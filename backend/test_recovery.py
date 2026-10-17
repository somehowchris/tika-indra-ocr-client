"""Real process-supervision tests; opt in with INDRA_TEST_MODEL_DIR.

These kill only test-owned children. They do not modify the supplied models.
"""

from __future__ import annotations

import http.client
import io
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest


@unittest.skipUnless(os.environ.get("INDRA_TEST_MODEL_DIR") and hasattr(signal, "SIGKILL"),
                     "set INDRA_TEST_MODEL_DIR on Linux for real supervision tests")
class RealRecovery(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.process = None
        self.stderr = (self.root / "service.log").open("wb")

    def tearDown(self):
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.stderr.close()
        self.temporary.cleanup()

    def start(self, models: Path, timeout="60"):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            self.port = listener.getsockname()[1]
        environment = {key: value for key, value in os.environ.items() if not key.startswith("INDRA_")}
        environment.update({
            "INDRA_MODEL_DIR": str(models), "INDRA_HOST": "127.0.0.1", "INDRA_PORT": str(self.port),
            "INDRA_WORKERS": "1", "INDRA_THREADS": "1", "INDRA_REQUEST_TIMEOUT_SECONDS": timeout,
        })
        self.process = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("server.py"))], env=environment,
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=self.stderr,
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                self.fail((self.root / "service.log").read_text())
            try:
                if self.request("GET", "/readyz")[0] == 200:
                    return
            except OSError:
                pass
            time.sleep(0.1)
        self.fail("test service did not become ready")

    def request(self, method, path, body=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        try:
            connection.request(method, path, body)
            response = connection.getresponse()
            return response.status, response.read()
        finally:
            connection.close()

    def worker_pid(self):
        for line in (self.root / "service.log").read_text().splitlines():
            record = json.loads(line)
            if record.get("event") == "ready":
                return record["worker_pids"][0]
        self.fail("test-owned OCR child not found")

    def test_idle_worker_death_exits_without_an_ocr_request(self):
        self.start(Path(os.environ["INDRA_TEST_MODEL_DIR"]))
        os.kill(self.worker_pid(), signal.SIGKILL)
        self.assertNotEqual(self.process.wait(timeout=5), 0)
        evidence = (self.root / "service.log").read_text()
        self.assertIn('"event":"supervisor_fatal"', evidence)
        self.assertIn('"reason":"worker_died"', evidence)

    def test_failed_replacement_exits_nonzero(self):
        import cv2
        import numpy as np
        from PIL import Image

        models = self.root / "models"
        models.mkdir()
        for path in Path(os.environ["INDRA_TEST_MODEL_DIR"]).iterdir():
            if path.is_file():
                # Independent directory entries; removing one cannot remove the
                # source model used by another test or a benchmark process.
                os.link(path, models / path.name)
        self.start(models, timeout="0.5")
        (models / "rec.onnx").unlink()
        image = np.full((1500, 1000), 255, np.uint8)
        for y in range(50, 1480, 40):
            cv2.putText(image, "Invoice record 12345 Total 250", (10, y), cv2.FONT_HERSHEY_SIMPLEX, 1, 0, 2)
        frame = Image.fromarray(image)
        encoded = io.BytesIO()
        frame.save(encoded, format="TIFF", save_all=True, append_images=[frame] * 15, compression="tiff_deflate")
        self.assertEqual(self.request("POST", "/ocr", encoded.getvalue())[0], 504)
        self.assertNotEqual(self.process.wait(timeout=10), 0)
        evidence = (self.root / "service.log").read_text()
        self.assertIn('"event":"worker_restart_failed"', evidence)
        self.assertIn('"reason":"worker_restart_failed"', evidence)


if __name__ == "__main__":
    unittest.main()
