#!/usr/bin/env python3
"""Bounded, offline Indra OCR image service. See backend/README.md."""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
import io
import json
import math
import multiprocessing as mp
import os
from pathlib import Path
import queue
import signal
import socket
from socketserver import ThreadingMixIn
import sys
import threading
import time
import uuid

from model_files import verify_models


def log(event: str, **fields) -> None:
    print(json.dumps({"event": event, **fields}, separators=(",", ":")), file=sys.stderr, flush=True)


def number(name: str, default: float, minimum: float, maximum: float) -> float:
    value = float(os.environ.get(name, default))
    if not math.isfinite(value) or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be between {minimum} and {maximum}")
    return value


def integer(name: str, default: int, minimum: int, maximum: int) -> int:
    value = number(name, default, minimum, maximum)
    if not value.is_integer():
        raise ValueError(f"{name} must be an integer")
    return int(value)


def boolean(name: str, default: bool = False) -> bool:
    value = os.environ.get(name, "1" if default else "0")
    if value not in ("0", "1"):
        raise ValueError(f"{name} must be 0 or 1")
    return value == "1"


@dataclass(frozen=True)
class Config:
    host: str
    port: int
    workers: int
    threads: int
    model_dir: Path
    manifest: Path
    math: bool
    batch_size: int
    batch_metrics: bool
    max_request: int
    max_pixels: int
    max_frames: int
    max_response: int
    queue_capacity: int
    queue_timeout: float
    read_timeout: float
    request_timeout: float
    startup_timeout: float

    @classmethod
    def from_env(cls) -> Config:
        config = cls(
            host=os.environ.get("INDRA_HOST", "127.0.0.1"),
            port=integer("INDRA_PORT", 8081, 1, 65535),
            workers=integer("INDRA_WORKERS", 1, 1, 2),
            threads=integer("INDRA_THREADS", 1, 1, 2),
            model_dir=Path(os.environ.get("INDRA_MODEL_DIR", "/models")),
            manifest=Path(os.environ.get("INDRA_MODEL_MANIFEST", Path(__file__).with_name("models.lock.json"))),
            math=boolean("INDRA_MATH"),
            batch_size=integer("INDRA_RECOGNITION_BATCH", 64, 1, 64),
            batch_metrics=boolean("INDRA_BATCH_METRICS"),
            max_request=integer("INDRA_MAX_REQUEST_BYTES", 20 * 1024 * 1024, 1, 20 * 1024 * 1024),
            max_pixels=integer("INDRA_MAX_PIXELS", 25_000_000, 1, 25_000_000),
            max_frames=integer("INDRA_MAX_FRAMES", 16, 1, 16),
            max_response=integer("INDRA_MAX_RESPONSE_BYTES", 4 * 1024 * 1024, 1, 4 * 1024 * 1024),
            queue_capacity=integer("INDRA_QUEUE_CAPACITY", 2, 0, 16),
            queue_timeout=number("INDRA_QUEUE_TIMEOUT_SECONDS", 2, 0.01, 30),
            read_timeout=number("INDRA_READ_TIMEOUT_SECONDS", 10, 0.05, 60),
            request_timeout=number("INDRA_REQUEST_TIMEOUT_SECONDS", 60, 0.1, 300),
            startup_timeout=number("INDRA_STARTUP_TIMEOUT_SECONDS", 120, 1, 300),
        )
        if config.workers * config.threads > 2:
            raise ValueError("INDRA_WORKERS * INDRA_THREADS must not exceed 2")
        return config


class ClientError(Exception):
    def __init__(self, status: int, message: str):
        super().__init__(message)
        self.status = status


class SessionMeter:
    """Optional actual tensor-shape measurements; forwards arrays without copying."""

    def __init__(self, session, stage: str, counts: Counter):
        self.session, self.stage, self.counts = session, stage, counts

    def run(self, names, inputs):
        tensor = inputs.get("x")
        if tensor is not None and tensor.ndim == 4:
            batch, _, height, width = tensor.shape
            valid = int(inputs["widths"].sum()) if "widths" in inputs else batch * width
            self.counts[(self.stage, batch, height, width, valid)] += 1
        return self.session.run(names, inputs)


def create_engine(config: Config, counts: Counter):
    # Imports must happen after thread and decoder limits are set in each child.
    import cv2
    from indra_ocr import OrtOCR

    class ConfiguredOCR(OrtOCR):
        def recognize(self, gray, boxes, batch=64):
            return super().recognize(gray, boxes, batch=config.batch_size)

    cv2.setNumThreads(config.threads)
    engine = ConfiguredOCR(
        config.model_dir,
        threads=config.threads,
        rec_int8=False,
        adaptive=True,
        providers=["CPUExecutionProvider"],
        furniture=True,
        math=config.math,
    )
    for name in ("det", "rec", "order", "orient", "script", "rec_cjk"):
        session = getattr(engine, name)
        if session is None or session.get_providers() != ["CPUExecutionProvider"]:
            raise ValueError(f"required CPU model unavailable: {name}")
    if config.batch_metrics:
        for name in ("rec", "rec_cjk", "script"):
            setattr(engine, name, SessionMeter(getattr(engine, name), name, counts))
        if engine.math is not None:
            engine.math.cls = SessionMeter(engine.math.cls, "math_classifier", counts)
            engine.math.enc = SessionMeter(engine.math.enc, "math_encoder", counts)
    return engine


def decode_images(payload: bytes, config: Config):
    """Read dimensions before decompression, then use upstream's grayscale decoder."""
    import cv2
    import numpy as np
    from PIL import Image, UnidentifiedImageError

    try:
        with Image.open(io.BytesIO(payload)) as header:
            if header.format not in ("PNG", "JPEG", "TIFF"):
                raise ClientError(415, "supported image formats: PNG, JPEG, TIFF")
            count, pixels = 0, 0
            while True:
                width, height = header.size
                count += 1
                pixels += width * height
                if count > config.max_frames or pixels > config.max_pixels:
                    raise ClientError(413, "image frame or pixel limit exceeded")
                try:
                    header.seek(count)
                except EOFError:
                    break
    except ClientError:
        raise
    except (UnidentifiedImageError, OSError, ValueError, Image.DecompressionBombError) as exc:
        raise ClientError(422, "invalid or oversized image") from exc

    compressed = np.frombuffer(payload, dtype=np.uint8)
    if count == 1:
        frame = cv2.imdecode(compressed, cv2.IMREAD_GRAYSCALE)
        frames = [] if frame is None else [frame]
    else:
        ok, frames = cv2.imdecodemulti(compressed, cv2.IMREAD_GRAYSCALE)
        if not ok:
            frames = []
    if len(frames) != count or any(frame.ndim != 2 for frame in frames):
        raise ClientError(422, "could not decode every image frame")
    actual_pixels = sum(frame.size for frame in frames)
    if actual_pixels > config.max_pixels:
        raise ClientError(413, "decoded image pixel limit exceeded")
    return frames, actual_pixels


def process_image(payload: bytes, config: Config, engine, counts: Counter):
    counts.clear()
    start = time.monotonic()
    frames, pixels = decode_images(payload, config)
    decoded = time.monotonic()
    output = bytearray()
    timings = []
    for index, gray in enumerate(frames):
        page = engine(gray)
        if page.source == "error":
            raise RuntimeError("Indra returned an error page")
        text = page.text.encode("utf-8")
        separator = b"\n\f\n" if index else b""
        if len(output) + len(separator) + len(text) > config.max_response:
            raise ClientError(413, "OCR response limit exceeded")
        output.extend(separator)
        output.extend(text)
        timings.append(page.timings)
    metrics = {
        "frames": len(frames),
        "pixels": pixels,
        "decode_ms": round((decoded - start) * 1000, 3),
        "processing_ms": round((time.monotonic() - start) * 1000, 3),
        "stages": timings,
    }
    if config.batch_metrics:
        metrics["batches"] = [
            {"stage": stage, "batch": batch, "height": height, "width": width, "valid_columns": valid, "calls": calls}
            for (stage, batch, height, width, valid), calls in sorted(counts.items())
        ]
    return bytes(output), metrics


def worker_main(connection, config: Config) -> None:
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    try:
        for name in ("OMP_NUM_THREADS", "OMP_THREAD_LIMIT", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS"):
            os.environ[name] = str(config.threads)
        os.environ["OPENCV_IO_MAX_IMAGE_PIXELS"] = str(config.max_pixels)
        os.environ["HF_HUB_OFFLINE"] = "1"
        import cv2
        import numpy as np
        from PIL import Image

        Image.MAX_IMAGE_PIXELS = config.max_pixels
        counts: Counter = Counter()
        engine = create_engine(config, counts)
        warm = np.full((240, 1000), 255, np.uint8)
        for y in (75, 150):
            cv2.putText(warm, "Indra OCR warm up 123", (30, y), cv2.FONT_HERSHEY_SIMPLEX, 1.3, 0, 2)
        engine(warm)
        connection.send(("ready", os.getpid()))
    except Exception as exc:
        log("worker_start_failed", error=type(exc).__name__, detail=str(exc)[:400])
        connection.send(("failed", type(exc).__name__))
        connection.close()
        return
    try:
        while True:
            try:
                payload = connection.recv_bytes(config.max_request)
            except EOFError:
                break
            try:
                body, metrics = process_image(payload, config, engine, counts)
                connection.send((200, body, metrics))
            except ClientError as exc:
                connection.send((exc.status, str(exc).encode("utf-8"), {}))
            except Exception as exc:
                log("worker_request_failed", error=type(exc).__name__, detail=str(exc)[:400])
                connection.send((500, b"OCR processing failed", {}))
    finally:
        connection.close()


class Worker:
    def __init__(self, config: Config):
        self.config = config
        self.process = None
        self.connection = None
        self.ready = False

    def start(self) -> None:
        context = mp.get_context("spawn")
        self.connection, child = context.Pipe()
        self.process = context.Process(target=worker_main, args=(child, self.config), daemon=True)
        self.process.start()
        child.close()
        try:
            if not self.connection.poll(self.config.startup_timeout):
                raise RuntimeError("model worker startup timed out")
            result = self.connection.recv()
            if result[0] != "ready":
                raise RuntimeError("model worker failed to start")
            self.ready = True
        except Exception:
            self.stop()
            raise

    def stop(self) -> None:
        self.ready = False
        if self.connection is not None:
            self.connection.close()
        if self.process is not None:
            if self.process.is_alive():
                self.process.terminate()
            self.process.join(timeout=1)
            if self.process.is_alive():
                self.process.kill()
                self.process.join(timeout=1)

    def call(self, payload: bytes, deadline: float):
        try:
            if not self.ready or not self.process.is_alive():
                raise EOFError("worker unavailable")
            # One explicit IPC copy into a persistent process. No per-image load.
            self.connection.send_bytes(payload)
            if not self.connection.poll(max(0.0, deadline - time.monotonic())):
                self.stop()
                raise ClientError(504, "OCR deadline exceeded")
            return self.connection.recv()
        except (EOFError, BrokenPipeError, OSError) as exc:
            self.stop()
            raise ClientError(503, "OCR worker unavailable") from exc


class WorkerPool:
    def __init__(self, config: Config):
        self.config = config
        self.closed = threading.Event()
        self.slots = [Worker(config) for _ in range(config.workers)]
        self.available: queue.Queue = queue.Queue(config.workers)
        self.admission = threading.BoundedSemaphore(config.workers + config.queue_capacity)
        try:
            for slot in self.slots:
                slot.start()
                self.available.put_nowait(slot)
        except Exception:
            self.close()
            raise

    def ready(self) -> bool:
        return not self.closed.is_set() and all(slot.ready and slot.process.is_alive() for slot in self.slots)

    def release(self, slot: Worker) -> None:
        if self.closed.is_set():
            slot.stop()
        elif slot.ready:
            self.available.put_nowait(slot)
        else:
            threading.Thread(target=self.restore, args=(slot,), daemon=True, name="worker-restart").start()

    def restore(self, slot: Worker) -> None:
        try:
            slot.start()
            self.release(slot)
            log("worker_restarted")
        except Exception as exc:
            # Remain unready; the orchestrator restarts the service. Do not spin.
            log("worker_restart_failed", error=type(exc).__name__)

    def close(self) -> None:
        self.closed.set()
        for slot in self.slots:
            slot.stop()


class BoundedServer(ThreadingMixIn, HTTPServer):
    daemon_threads = True
    request_queue_size = 8

    def __init__(self, config: Config, pool: WorkerPool):
        self.config, self.pool = config, pool
        # Separate spare handlers keep probes responsive during OCR and queuing.
        self.handlers = threading.BoundedSemaphore(config.workers + config.queue_capacity + 4)
        super().__init__((config.host, config.port), Handler)

    def process_request(self, request, client_address):
        if not self.handlers.acquire(blocking=False):
            try:
                request.settimeout(0.1)
                request.sendall(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy")
            except OSError:
                pass  # No response can be delivered to a disconnected client.
            finally:
                self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except Exception:
            self.handlers.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self.handlers.release()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "IndraImageService/1"
    sys_version = ""

    def setup(self) -> None:
        self.request.settimeout(self.server.config.read_timeout)
        super().setup()

    def reply(self, status: int, body: bytes) -> None:
        self.close_connection = True
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.send_header("Cache-Control", "no-store")
        if status == 503:
            self.send_header("Retry-After", "1")
        self.end_headers()
        self.wfile.write(body)

    def handle_expect_100(self) -> bool:
        self.reply(417, b"Expect is unsupported; send a bounded Content-Length body")
        return False

    def do_GET(self) -> None:
        if self.path == "/healthz":
            self.reply(200, b"alive\n")
        elif self.path == "/readyz":
            ready = self.server.pool.ready()
            self.reply(200 if ready else 503, b"ready\n" if ready else b"unready\n")
        else:
            self.reply(404, b"not found")

    def body_length(self) -> int:
        if self.headers.get_all("Transfer-Encoding"):
            raise ClientError(400, "Transfer-Encoding is unsupported")
        if self.headers.get("Content-Encoding", "identity") != "identity":
            raise ClientError(415, "Content-Encoding is unsupported")
        lengths = self.headers.get_all("Content-Length", [])
        if not lengths:
            raise ClientError(411, "Content-Length is required")
        if len(lengths) != 1 or not lengths[0].isascii() or not lengths[0].isdecimal():
            raise ClientError(400, "one decimal Content-Length is required")
        length = int(lengths[0])
        if not 0 < length <= self.server.config.max_request:
            raise ClientError(413, "request byte limit exceeded")
        return length

    def read_body(self, length: int, deadline: float) -> bytes:
        data = bytearray()
        while len(data) < length:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ClientError(408, "request body deadline exceeded")
            self.connection.settimeout(remaining)
            chunk = self.rfile.read1(min(64 * 1024, length - len(data)))
            if not chunk:
                raise ClientError(400, "truncated request body")
            data.extend(chunk)
        return bytes(data)

    def do_POST(self) -> None:
        if self.path != "/ocr":
            self.reply(404, b"not found")
            return
        config, pool = self.server.config, self.server.pool
        start = time.monotonic()
        request_id = uuid.uuid4().hex
        admitted, slot = False, None
        status, response, metrics, length = 500, b"request failed", {}, 0
        try:
            length = self.body_length()
            admitted = pool.admission.acquire(blocking=False)
            if not admitted:
                raise ClientError(503, "OCR queue full")
            payload = self.read_body(length, min(start + config.request_timeout, start + config.read_timeout))
            queued = time.monotonic()
            try:
                slot = pool.available.get(timeout=min(config.queue_timeout, max(0.001, start + config.request_timeout - queued)))
            except queue.Empty as exc:
                raise ClientError(503, "OCR queue deadline exceeded") from exc
            dispatched = time.monotonic()
            status, response, metrics = slot.call(payload, start + config.request_timeout)
            metrics["queue_ms"] = round((dispatched - queued) * 1000, 3)
            metrics["worker_pid"] = slot.process.pid
        except ClientError as exc:
            status, response = exc.status, str(exc).encode("utf-8")
        except (TimeoutError, socket.timeout):
            status, response = 408, b"request body deadline exceeded"
        except (BrokenPipeError, ConnectionResetError):
            status, response = 499, b"client disconnected"
        except Exception as exc:
            log("request_failed", request_id=request_id, error=type(exc).__name__)
        finally:
            if slot is not None:
                pool.release(slot)
            if admitted:
                pool.admission.release()
        try:
            self.connection.settimeout(config.read_timeout)
            self.reply(status, response)
        except (OSError, TimeoutError):
            log("response_disconnected", request_id=request_id)
        log(
            "ocr_request", request_id=request_id, status=status,
            input_bytes=length, output_bytes=len(response),
            wall_ms=round((time.monotonic() - start) * 1000, 3),
            math=config.math, recognition_batch=config.batch_size, **metrics,
        )

    def log_message(self, format, *args) -> None:
        # Request metrics above deliberately omit document text and client URLs.
        return


def main() -> None:
    config = Config.from_env()
    lock = verify_models(config.model_dir, config.manifest, config.math)
    pool = WorkerPool(config)
    try:
        with BoundedServer(config, pool) as server:
            def stop(signum, frame):
                threading.Thread(target=server.shutdown, daemon=True).start()

            signal.signal(signal.SIGTERM, stop)
            signal.signal(signal.SIGINT, stop)
            log("ready", workers=config.workers, threads=config.threads, math=config.math,
                recognition_batch=config.batch_size, model_revision=lock["revision"])
            server.serve_forever(poll_interval=0.1)
    finally:
        pool.close()


if __name__ == "__main__":
    main()
