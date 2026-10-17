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


def split_service_log(text: str) -> tuple[list[dict], str]:
    """Separate native-library stderr from the backend's JSON event records.

    JSON-looking lines are parsed strictly. A malformed event must fail the
    test instead of disappearing inside a broad JSONDecodeError handler.
    Well-formed third-party JSON without an event remains a diagnostic.
    """
    events, diagnostics = [], []
    for line in text.splitlines(keepends=True):
        if not line.lstrip().startswith("{"):
            diagnostics.append(line)
            continue
        record = json.loads(line)
        if "event" not in record:
            diagnostics.append(line)
            continue
        if not isinstance(record["event"], str) or not record["event"]:
            raise ValueError("structured service event requires a nonempty event name")
        events.append(record)
    return events, "".join(diagnostics)


class ServiceLogBoundary(unittest.TestCase):
    def test_plain_and_third_party_json_diagnostics_are_preserved(self):
        diagnostics = 'native library warning\n{"library":"diagnostic"}\n'
        events, stderr = split_service_log(diagnostics + '{"event":"ready","worker_pids":[123]}\n')
        self.assertEqual(stderr, diagnostics)
        self.assertEqual(events, [{"event": "ready", "worker_pids": [123]}])

    def test_malformed_structured_records_are_not_suppressed(self):
        for text in ('{"event":"ready"\n', '{ "event" "ready" }\n'):
            with self.subTest(text=text), self.assertRaises(json.JSONDecodeError):
                split_service_log("native library warning\n" + text)

    def test_invalid_event_names_are_not_suppressed(self):
        for value in (None, "", 123, []):
            with self.subTest(value=value), self.assertRaises(ValueError):
                split_service_log(json.dumps({"event": value}) + "\n")


@unittest.skipUnless(os.environ.get("INDRA_TEST_MODEL_DIR") and hasattr(signal, "SIGKILL"),
                     "set INDRA_TEST_MODEL_DIR on Linux for real supervision tests")
class RealRecovery(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.process = None
        self.logs = self.root
        if os.environ.get("INDRA_TEST_LOG_DIR"):
            self.logs = Path(os.environ["INDRA_TEST_LOG_DIR"]) / self._testMethodName
            self.logs.mkdir(parents=True, exist_ok=False)
        self.raw_log = self.logs / "service.stderr"
        self.stderr = self.raw_log.open("wb")

    def tearDown(self):
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.stderr.close()
        try:
            self.events()
        finally:
            self.temporary.cleanup()

    def events(self):
        events, diagnostics = split_service_log(self.raw_log.read_text())
        (self.logs / "events.jsonl").write_text("".join(json.dumps(record) + "\n" for record in events))
        (self.logs / "diagnostics.stderr").write_text(diagnostics)
        return events

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
                self.fail(self.raw_log.read_text())
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
        for record in self.events():
            if record.get("event") == "ready":
                pids = record["worker_pids"]
                self.assertEqual(len(pids), 1)
                self.assertIs(type(pids[0]), int)
                self.assertGreater(pids[0], 0)
                return pids[0]
        self.fail("test-owned OCR child not found")

    def test_idle_worker_death_exits_without_an_ocr_request(self):
        self.start(Path(os.environ["INDRA_TEST_MODEL_DIR"]))
        os.kill(self.worker_pid(), signal.SIGKILL)
        self.assertNotEqual(self.process.wait(timeout=5), 0)
        self.assertTrue(any(record.get("event") == "supervisor_fatal" and record.get("reason") == "worker_died"
                            for record in self.events()))

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
        events = self.events()
        self.assertTrue(any(record.get("event") == "worker_restart_failed" for record in events))
        self.assertTrue(any(record.get("event") == "supervisor_fatal" and record.get("reason") == "worker_restart_failed"
                            for record in events))


if __name__ == "__main__":
    unittest.main()
