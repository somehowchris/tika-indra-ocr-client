"""Invariant tests, plus real HTTP tests when INDRA_TEST_URL is set.

    python -m unittest discover -s backend -p 'test_*.py' -v
"""

from __future__ import annotations

from dataclasses import replace
import hashlib
import http.client
import io
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import urlsplit
import zlib

import numpy as np
from PIL import Image

from model_files import verify_models
from server import ClientError, Config, decode_images, process_image


DATA = Path(__file__).parent / "tests" / "data"


def oversized_png() -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack("!I", len(data)) + kind + data + struct.pack("!I", zlib.crc32(kind + data))

    # Deliberately only a header: rejection must precede pixel decoding.
    header = struct.pack("!IIBBBBB", 5001, 5000, 8, 0, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", b"") + chunk(b"IEND", b"")


class InputInvariants(unittest.TestCase):
    def setUp(self):
        with patch.dict(os.environ, {}, clear=True):
            self.config = Config.from_env()

    def test_dimensions_checked_before_decompression(self):
        with patch("cv2.imdecode", side_effect=AssertionError("decoder must not run")):
            with self.assertRaises(ClientError) as failure:
                decode_images(oversized_png(), self.config)
        self.assertEqual(failure.exception.status, 413)

    def test_tiff_frame_limit_checked_before_decompression(self):
        image = Image.new("L", (8, 8), 255)
        data = io.BytesIO()
        image.save(data, format="TIFF", save_all=True, append_images=[image] * 16)
        with patch("cv2.imdecodemulti", side_effect=AssertionError("decoder must not run")):
            with self.assertRaises(ClientError) as failure:
                decode_images(data.getvalue(), self.config)
        self.assertEqual(failure.exception.status, 413)

    def test_unsupported_format_rejected(self):
        data = io.BytesIO()
        Image.new("L", (8, 8), 255).save(data, format="BMP")
        with self.assertRaises(ClientError) as failure:
            decode_images(data.getvalue(), self.config)
        self.assertEqual(failure.exception.status, 415)

    def test_response_is_never_silently_truncated(self):
        class Page:
            source = "ocr"
            text = "x" * 101
            timings = {}

        with patch("server.decode_images", return_value=([np.zeros((1, 1), np.uint8)], 1)):
            with self.assertRaises(ClientError) as failure:
                process_image(b"x", replace(self.config, max_response=100), lambda _: Page(), {})
        self.assertEqual(failure.exception.status, 413)

    def test_upstream_error_page_is_not_success(self):
        class Page:
            source = "error"

        with patch("server.decode_images", return_value=([np.zeros((1, 1), np.uint8)], 1)):
            with self.assertRaises(RuntimeError):
                process_image(b"x", self.config, lambda _: Page(), {})

    def test_resource_oversubscription_rejected(self):
        with patch.dict(os.environ, {"INDRA_WORKERS": "2", "INDRA_THREADS": "2"}, clear=True):
            with self.assertRaises(ValueError):
                Config.from_env()

    def test_model_corruption_detected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            content = b"expected"
            (root / "det.onnx").write_bytes(content)
            lock = {
                "repository": "datalab-to/indra-ocr", "revision": "1" * 40,
                "files": [{"name": "det.onnx", "profile": "base", "size": len(content),
                           "sha256": hashlib.sha256(content).hexdigest()}],
            }
            manifest = root / "models.lock.json"
            manifest.write_text(json.dumps(lock))
            verify_models(root, manifest, False)
            (root / "det.onnx").write_bytes(b"modified")
            with self.assertRaisesRegex(ValueError, "checksum"):
                verify_models(root, manifest, False)


@unittest.skipUnless(os.environ.get("INDRA_TEST_URL"), "set INDRA_TEST_URL for a running real backend")
class RealService(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        address = urlsplit(os.environ["INDRA_TEST_URL"])
        if address.scheme != "http" or address.hostname not in ("127.0.0.1", "localhost"):
            raise ValueError("integration tests require an explicit loopback HTTP service")
        cls.address = (address.hostname, address.port or 8081)

    def call(self, method, path, body=None, headers=None):
        client = http.client.HTTPConnection(*self.address, timeout=20)
        try:
            client.request(method, path, body, headers or {})
            response = client.getresponse()
            data = response.read()
            self.assertEqual(response.getheader("Connection"), "close")
            self.assertEqual(int(response.getheader("Content-Length")), len(data))
            return response.status, data
        finally:
            client.close()

    def raw(self, request: bytes):
        with socket.create_connection(self.address, timeout=10) as client:
            client.sendall(request)
            client.shutdown(socket.SHUT_WR)
            response = http.client.HTTPResponse(client)
            response.begin()
            data = response.read()
            self.assertEqual(response.getheader("Connection"), "close")
            self.assertEqual(int(response.getheader("Content-Length")), len(data))
            return response.status, data

    def test_health_and_readiness(self):
        self.assertEqual(self.call("GET", "/healthz"), (200, b"alive\n"))
        self.assertEqual(self.call("GET", "/readyz"), (200, b"ready\n"))

    def test_png_and_tiff_all_frames(self):
        for name, expected in (("smoke.png", [b"INVOICE 12345"]),
                               ("two-pages.tiff", [b"INVOICE 12345", b"SECOND PAGE 67890", b"\x0c"])):
            with self.subTest(name=name):
                status, text = self.call("POST", "/ocr", (DATA / name).read_bytes())
                self.assertEqual(status, 200)
                for value in expected:
                    self.assertIn(value, text)

    def test_invalid_image_returns_non_success(self):
        self.assertEqual(self.call("POST", "/ocr", b"not an image")[0], 422)

    def test_pixel_limit(self):
        self.assertEqual(self.call("POST", "/ocr", oversized_png())[0], 413)

    def test_body_framing_errors(self):
        requests = [
            (411, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\n\r\n"),
            (400, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx"),
            (400, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nContent-Length: 1\r\n\r\nx"),
            (400, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Length: -1\r\n\r\n"),
            (413, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Length: 20971521\r\n\r\n"),
            (400, b"POST /ocr HTTP/1.1\r\nHost: localhost\r\nContent-Length: 10\r\n\r\nx"),
        ]
        for status, request in requests:
            with self.subTest(status=status, request=request[:90]):
                self.assertEqual(self.raw(request)[0], status)


if __name__ == "__main__":
    unittest.main()
