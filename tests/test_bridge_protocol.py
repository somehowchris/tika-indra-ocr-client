"""Real adapter and startup probes + controlled HTTP peers; no OCR measurement."""

from contextlib import contextmanager
import importlib.util
from pathlib import Path
import os
import socket
import socketserver
import struct
import subprocess
import tempfile
import threading
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
BRIDGE = Path(os.environ.get("BRIDGE_BIN", ROOT / "target/x86_64-unknown-linux-gnu/release/indra-tesseract"))
SMOKE_SPEC = importlib.util.spec_from_file_location("smoke_stack", ROOT / "scripts/smoke_stack.py")
SMOKE = importlib.util.module_from_spec(SMOKE_SPEC)
SMOKE_SPEC.loader.exec_module(SMOKE)


@contextmanager
def peer(reply, captured):
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            stream = self.request.makefile("rb")
            first = stream.readline()
            length = 0
            while True:
                line = stream.readline()
                if line == b"\r\n":
                    break
                if not line:
                    return
                if line.lower().startswith(b"content-length:"):
                    length = int(line.split(b":", 1)[1])
            captured.append((first, stream.read(length)))
            try:
                if callable(reply):
                    reply(self.request)
                else:
                    self.request.sendall(reply)
            except (BrokenPipeError, ConnectionResetError):
                pass  # Expected when exercising the client's deadline.

    class Server(socketserver.ThreadingTCPServer):
        daemon_threads = True
        allow_reuse_address = True

    with Server(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
        try:
            yield f"127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            thread.join(timeout=2)


def http(body: bytes) -> bytes:
    return b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: " + str(len(body)).encode("ascii") + b"\r\nConnection: close\r\n\r\n" + body


def reset(connection):
    connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    connection.close()


class BridgeProtocol(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="indra bridge ")
        self.addCleanup(self.temp.cleanup)
        self.input = Path(self.temp.name) / "page with spaces.png"
        self.base = Path(self.temp.name) / "output base"
        self.output = Path(str(self.base) + ".txt")
        self.payload = bytes(range(256)) * 512
        self.input.write_bytes(self.payload)

    def run_bridge(self, address, extra_env=None):
        environment = dict(os.environ)
        environment.update(INDRA_ADDR=address)
        if extra_env:
            environment.update(extra_env)
        return subprocess.run(
            [str(BRIDGE), str(self.input), str(self.base), "--psm", "3", "-c", "page_separator=", "-c", "preserve_interword_spaces=0", "txt"],
            env=environment, capture_output=True, timeout=5,
        )

    def test_complete_payload_and_utf8_output(self):
        expected = "Zürich — café. 日本語\n".encode("utf-8")
        captured = []
        with peer(http(expected), captured) as address:
            result = self.run_bridge(address)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_bytes(), expected)
        self.assertEqual(captured, [(b"POST /ocr HTTP/1.1\r\n", self.payload)])

    def test_backend_failure_never_creates_output(self):
        responses = [
            b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\nshort",
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4294967296\r\n\r\n",
            http(b"\xff\xfe"),
        ]
        for reply in responses:
            with self.subTest(reply=reply), peer(reply, []) as address:
                result = self.run_bridge(address)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.output.exists())

    def test_total_deadline_cannot_be_extended_by_drip_response(self):
        def drip(connection):
            connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 20\r\n\r\n")
            for _ in range(20):
                time.sleep(0.2)
                connection.sendall(b"x")

        with peer(drip, []) as address:
            started = time.monotonic()
            result = self.run_bridge(address, {"INDRA_TIMEOUT_SECONDS": "1"})
            elapsed = time.monotonic() - started
        self.assertNotEqual(result.returncode, 0)
        self.assertLess(elapsed, 1.6)
        self.assertFalse(self.output.exists())

    def test_existing_output_is_not_overwritten(self):
        self.output.write_text("keep this", encoding="utf-8")
        with peer(http(b"replacement"), []) as address:
            result = self.run_bridge(address)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.output.read_text(), "keep this")

    def test_valid_blank_page_is_successful_empty_file(self):
        with peer(http(b""), []) as address:
            result = self.run_bridge(address)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_bytes(), b"")

    def test_availability_probe_does_not_require_backend(self):
        for args in ([], ["--version"]):
            result = subprocess.run([str(BRIDGE), *args], capture_output=True, timeout=2)
            self.assertEqual(result.returncode, 0)
            self.assertIn(b"indra-tesseract", result.stdout)

    def test_health_calls_ready_endpoint(self):
        captured = []
        with peer(http(b"ready\n"), captured) as address:
            result = subprocess.run([str(BRIDGE), "--ready"], env={**os.environ, "INDRA_ADDR": address}, capture_output=True, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(captured[0][0], b"GET /readyz HTTP/1.1\r\n")

    def test_oversized_input_fails_before_network(self):
        with self.input.open("wb") as file:
            file.truncate(20 * 1024 * 1024 + 1)
        result = self.run_bridge("127.0.0.1:1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"20 MiB", result.stderr)
        self.assertFalse(self.output.exists())

    def test_combined_health_requires_tika_too(self):
        for tika_reply, expected in [(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", 0), (b"HTTP/1.1 503 Busy\r\n\r\n", 1)]:
            with peer(http(b"ready\n"), []) as indra, peer(tika_reply, []) as tika:
                result = subprocess.run([str(BRIDGE), "--tika-ready"], env={**os.environ, "INDRA_ADDR": indra, "TIKA_ADDR": tika}, capture_output=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)

    def test_tika_server_health_does_not_require_indra(self):
        replies = [
            (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", 0),
            (b"HTTP/1.1 503 Busy\r\n\r\n", 1),
            (b"not HTTP\r\n", 1),
            (reset, 1),
        ]
        for reply, expected in replies:
            captured = []
            with self.subTest(reply=reply), peer(reply, captured) as tika:
                result = subprocess.run(
                    [str(BRIDGE), "--tika-server-ready"],
                    env={**os.environ, "INDRA_ADDR": "deliberately-invalid", "TIKA_ADDR": tika},
                    capture_output=True, timeout=3,
                )
            self.assertEqual(result.returncode, expected, result.stderr)
            self.assertEqual(captured, [(b"GET /version HTTP/1.1\r\n", b"")])

    def test_remote_address_is_rejected(self):
        result = self.run_bridge("192.0.2.1:8081")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"loopback", result.stderr)


class StackReadiness(unittest.TestCase):
    def test_connection_reset_is_retried_only_during_startup(self):
        captured = []

        def restart_then_ready(connection):
            if len(captured) == 1:
                reset(connection)
            else:
                connection.sendall(http(b"Apache Tika 3.3.2"))

        with peer(restart_then_ready, captured) as tika:
            self.assertEqual(SMOKE.wait_for_tika(f"http://{tika}", 2), "Apache Tika 3.3.2")
        self.assertEqual(len(captured), 2)
        self.assertTrue(all(first.startswith(b"GET /version ") for first, _ in captured))
        captured = []
        with peer(reset, captured) as tika:
            with self.assertRaises(ConnectionError):
                SMOKE.request(f"http://{tika}", "/tika", b"image", "image/png")
        self.assertEqual(len(captured), 1, "extraction failures must not be retried")

    def test_wrong_version_is_not_retried(self):
        captured = []
        with peer(http(b"Apache Tika 3.3.1"), captured) as tika:
            with self.assertRaisesRegex(RuntimeError, "expected Tika 3.3.2"):
                SMOKE.wait_for_tika(f"http://{tika}", 2)
        self.assertEqual(len(captured), 1)

    def test_non_ready_status_expires_at_the_startup_deadline(self):
        with peer(b"HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n", []) as tika:
            started = time.monotonic()
            with self.assertRaisesRegex(TimeoutError, "startup deadline"):
                SMOKE.wait_for_tika(f"http://{tika}", 0.2)
            self.assertLess(time.monotonic() - started, 0.7)


if __name__ == "__main__":
    unittest.main(verbosity=2)
