"""Reference-only evidence harness. Never imported by the Rust service.

Runs the unmodified, hash-locked upstream Python implementation. Wrappers record
real arguments/results; they do not replace OCR algorithms. Timings here are
diagnostic only: this local environment has no controlled CPU-quota attestation.
"""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import traceback

ROOT = Path("/workspace/scratch/064a1be410c8")
REPO = ROOT / "repo"
UPSTREAM = ROOT / "indra-upstream"
MODELS = ROOT / "model-reference"
EVIDENCE = ROOT / "python-reference"
REVISION = "3d773aeedc5d79d1d0a2a04007a9075cd71a59f4"
THREAD_VARIABLES = (
    "OMP_NUM_THREADS", "OMP_THREAD_LIMIT", "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS", "VECLIB_MAXIMUM_THREADS",
)
for key in THREAD_VARIABLES:
    if os.environ.get(key) != "1":
        raise RuntimeError(f"{key}=1 must be set before importing numerical libraries")
os.environ["HF_HUB_OFFLINE"] = "1"

import cv2
import numpy as np
import onnxruntime as ort
import indra_ocr.pipeline as pipeline


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def dump(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def verify_environment() -> dict:
    model_lock = REPO / "backend/models.lock.json"
    lock = json.loads(model_lock.read_text())
    for artifact in lock["files"]:
        file = MODELS / artifact["name"]
        if file.stat().st_size != artifact["size"] or sha(file) != artifact["sha256"]:
            raise RuntimeError(f"model artifact verification failed: {artifact['name']}")
    actual_revision = subprocess.check_output(
        ["git", "-C", str(UPSTREAM), "rev-parse", "HEAD"], text=True
    ).strip()
    if actual_revision != REVISION:
        raise RuntimeError("reference commit mismatch")
    package = Path(pipeline.__file__).parent
    source_hashes = {}
    for file in sorted(package.rglob("*.py")):
        relative = file.relative_to(package)
        source = UPSTREAM / "indra_ocr" / relative
        if not source.is_file() or file.read_bytes() != source.read_bytes():
            raise RuntimeError(f"installed reference source differs from commit: {relative}")
        source_hashes[str(relative)] = sha(file)
    versions = {
        distribution.metadata["Name"]: distribution.version
        for distribution in importlib.metadata.distributions()
    }
    if versions.get("indra-ocr") != "0.3.2" or np.__version__ != "2.5.4" or ort.__version__ != "1.31.0":
        raise RuntimeError("unexpected locked numerical/reference package version")
    cv2.setNumThreads(1)
    return {
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "purpose": "fresh implementation parity; not a quality estimate or resource-attested performance measurement",
        "python": sys.version,
        "python_executable_sha256": sha(Path(sys.executable).resolve()),
        "packages": versions,
        "requirements_lock_sha256": sha(REPO / "backend/requirements.lock"),
        "harness_sha256": sha(Path(__file__)),
        "model_lock_sha256": sha(model_lock),
        "model_snapshot": lock,
        "upstream_commit": actual_revision,
        "installed_source_hashes": source_hashes,
        "machine": platform.machine(),
        "kernel": platform.release(),
        "affinity": sorted(os.sched_getaffinity(0)),
        "cpu_quota_attested": False,
        "thread_environment": {key: os.environ[key] for key in THREAD_VARIABLES},
        "opencv_threads": cv2.getNumThreads(),
        "available_providers": ort.get_available_providers(),
        "ort_build": ort.get_build_info(),
        "settings": {
            "threads": 1, "cv_threads": 1, "rec_int8": False,
            "adaptive": True, "table_heads": False, "batch_size": 64,
            "intra_op_spinning": True, "inter_op_spinning": True,
            "execution_mode": "sequential", "inter_op_threads": 1,
            "graph_optimization": "all", "cpu_arena": False,
            "memory_pattern": False, "providers": ["CPUExecutionProvider"],
            "page_separator": "\n\f\n",
            "reference_option_override": "A SessionOptions factory only sets memory_pattern=False and explicit spinning flags before the unchanged upstream constructor; Python otherwise leaves memory patterns enabled by default.",
        },
    }


def plan() -> dict:
    corpus = json.loads((REPO / "evaluation/corpus-frozen.json").read_text())
    by_id = {case["id"]: case for case in corpus["cases"]}
    selected = [
        "bhl-deu-0018", "eurlex-ai-act-el-p0002",
        "bongabdo-img_20230101_0016", "sankaracarya1925-03-body",
        "jingbao-1930-500", "AMSMB_1-3-100", "lectaurep-riant-01",
    ]
    cases = []

    def add(identifier, path, purpose, math=False, expected=None):
        digest = sha(path)
        if expected is not None and digest != expected:
            raise RuntimeError(f"frozen input mismatch: {identifier}")
        cases.append({
            "id": identifier, "path": str(path), "input_sha256": digest,
            "input_bytes": path.stat().st_size, "math": math, "purpose": purpose,
        })

    add("smoke", REPO / "backend/tests/data/smoke.png", "short PNG decoding and exact integration text")
    add("two-pages", REPO / "backend/tests/data/two-pages.tiff", "classic TIFF frame decoding and page boundary")
    add("jpeg", REPO / "benchmarks/results/synthetic-fixtures/jpeg_200.jpg", "JPEG decoder parity; synthetic integration fixture")
    add("skew", REPO / "benchmarks/results/synthetic-fixtures/skew_200.png", "deskew path; synthetic integration fixture")
    for identifier in selected:
        case = by_id[identifier]
        add(identifier, ROOT / "corpus-data" / case["input"]["path"],
            f"prespecified representative: {case['script']}, {case['domain']}, {case['task']}",
            expected=case["input"]["sha256"])
    # Rotation is a declared transform for implementation acceptance, not another
    # independent quality case. Both implementations receive the same encoded file.
    source = ROOT / "corpus-data" / by_id["eurlex-ai-act-el-p0002"]["input"]["path"]
    rotated = EVIDENCE / "diagnostics-rotated-greek.png"
    gray = cv2.imread(str(source), cv2.IMREAD_GRAYSCALE)
    if gray is None or not cv2.imwrite(str(rotated), cv2.rotate(gray, cv2.ROTATE_90_CLOCKWISE)):
        raise RuntimeError("rotation fixture generation failed")
    add("rotated-greek", rotated, "one clockwise quarter-turn of the frozen Greek legal page, using OpenCV5.0.0.93")
    add("math-off", EVIDENCE / "diagnostics-math.png", "synthetic LaTeX equation page at 200 dpi; optional math disabled")
    add("math-on", EVIDENCE / "diagnostics-math.png", "same synthetic LaTeX equation page; classifier and actual autoregressive math path", math=True)
    return {
        "created_before_inference": True,
        "selection_policy": "Explicit short transport cases, seven script/domain representatives, rotation derivative and one synthetic math pair; selected before inspecting any model outputs.",
        "corpus_manifest_sha256": sha(REPO / "evaluation/corpus-frozen.json"),
        "math_source_sha256": sha(EVIDENCE / "diagnostics-math.tex"),
        "math_rasterization": "pdfTeX1.40.25/TeXLive2023, extarticle14pt+amsmath+amssymb; pdftoppm -png -gray -r200 -singlefile",
        "cases": cases,
    }


class Recorder:
    def __init__(self, path: Path):
        path.mkdir()
        self.path = path
        self.events = []
        self.total = 0

    def save(self, name: str, value) -> None:
        array = np.ascontiguousarray(value)
        if self.total + array.nbytes > 512 * 1024 * 1024:
            raise RuntimeError("reference tensor evidence exceeded 512 MiB")
        file = self.path / f"{len(self.events):06}-{name}.npy"
        np.save(file, array, allow_pickle=False)
        self.events.append({
            "name": name, "file": file.name, "shape": list(array.shape),
            "dtype": array.dtype.str, "sha256": sha(file), "bytes": file.stat().st_size,
        })
        self.total += array.nbytes

    def close(self) -> None:
        dump(self.path / "manifest.json", {"data_bytes": self.total, "events": self.events})


INPUT_NAMES = {
    "orient": {"x": "orientation-input"},
    "det": {"x": "detection-input"},
    "script": {"x": "script-input", "widths": "line-widths"},
    "rec": {"x": "text-input", "widths": "line-widths"},
    "rec_cjk": {"x": "chinese-input", "widths": "line-widths"},
    "order": {"feats": "order-features"},
    "mathcls": {"x": "math-classifier-input", "widths": "line-widths"},
    "math_enc": {"x": "math-input", "widths": "math-widths"},
    "math_init": {}, "math_step": {},
}
OUTPUT_NAMES = {
    "orient": ["orientation-output"], "det": ["detection-output"],
    "script": ["script-probabilities"],
    "rec": ["text-ids", "text-confidences", "ctc-lengths"],
    "rec_cjk": ["chinese-ids", "chinese-confidences", "ctc-lengths"],
    "order": ["order-pairwise", "order-classes", "order-same-block"],
    "mathcls": ["math-classifier-probabilities"],
    "math_enc": ["math-memory", "math-memory-mask"],
    "math_init": ["math-cross-k", "math-cross-v"],
    "math_step": ["math-logits"],
}


class SessionRecorder:
    def __init__(self, session, model, owner):
        self.session, self.model, self.owner = session, model, owner

    def __getattr__(self, name):
        return getattr(self.session, name)

    def run(self, names, feeds, *args):
        recorder = self.owner.recorder
        if recorder is None:
            return self.session.run(names, feeds, *args)
        for original, name in INPUT_NAMES[self.model].items():
            recorder.save(name, feeds[original])
        outputs = self.session.run(names, feeds, *args)
        for name, value in zip(OUTPUT_NAMES[self.model], outputs):
            recorder.save(name, value)
        return outputs


class InstrumentedReference:
    def __init__(self, math: bool):
        self.recorder = None
        self.details = []
        original = ort.SessionOptions

        def options():
            result = original()
            result.enable_mem_pattern = False
            result.add_session_config_entry("session.intra_op.allow_spinning", "1")
            result.add_session_config_entry("session.inter_op.allow_spinning", "1")
            return result

        ort.SessionOptions = options
        try:
            self.engine = pipeline.OrtOCR(
                MODELS, threads=1, rec_int8=False, adaptive=True,
                table_heads=False, math=math, providers=["CPUExecutionProvider"],
            )
        finally:
            ort.SessionOptions = original
        for name in ("orient", "det", "script", "rec", "rec_cjk", "order"):
            setattr(self.engine, name, SessionRecorder(getattr(self.engine, name), name, self))
        if math:
            for name, label in (("cls", "mathcls"), ("enc", "math_enc"), ("init", "math_init"), ("step", "math_step")):
                setattr(self.engine.math, name, SessionRecorder(getattr(self.engine.math, name), label, self))
            original_read = self.engine.math.read
            math_code = original_read.__func__.__code__
            step_end = next(
                number for number, line in enumerate(Path(math_code.co_filename).read_text().splitlines(), 1)
                if line.strip() == "if done.all():"
            )

            def math_read(*args, **kwargs):
                if self.recorder is None:
                    return original_read(*args, **kwargs)
                # Capture actual upstream post-update loop locals at its stop
                # check. No duplicate stopping algorithm and no retained frames.
                def observe(frame, event, _argument):
                    if frame.f_code is math_code:
                        if event == "line" and frame.f_lineno == step_end:
                            self.recorder.save("math-next-token", frame.f_locals["tok"])
                            self.recorder.save("math-done", frame.f_locals["done"])
                        return observe
                    return None

                prior = sys.gettrace()
                sys.settrace(observe)
                try:
                    result = original_read(*args, **kwargs)
                    self.details.append({"math_readings": result})
                    return result
                finally:
                    sys.settrace(prior)

            self.engine.math.read = math_read
        original_upright = self.engine.upright

        def upright(*args, **kwargs):
            result = original_upright(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("turn-skew", np.asarray([result[1], result[2]], dtype=np.float64))
                self.recorder.save("upright", result[0])
            return result

        self.engine.upright = upright
        original_at = self.engine._detect_at

        def detect_at(*args, **kwargs):
            result = original_at(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("detection-boxes-unscaled", result[0][:, :4])
            return result

        self.engine._detect_at = detect_at
        original_detect = self.engine.detect

        def detect(*args, **kwargs):
            result = original_detect(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("detection-boxes", result[:, :4])
            return result

        self.engine.detect = detect
        original_order = self.engine.reading_order

        def reading_order(boxes, texts, wh):
            result = original_order(boxes, texts, wh)
            if self.recorder is not None:
                if len(boxes) >= 2:
                    self.recorder.save("order-permutation", np.asarray(result[0], dtype=np.int64))
                self.details.append({"split_boxes": boxes.tolist(), "split_texts": texts, "order": result[0]})
            return result

        self.engine.reading_order = reading_order

    def read(self, path: Path, recorder: Recorder | None):
        self.recorder = recorder
        self.details = []
        pages = []
        for gray in pipeline.read_images(path):
            if recorder is not None:
                recorder.save("decoded", gray)
            page = self.engine(gray)
            pages.append(page.text)
            self.details.append({"rotation": page.rotation, "skew": page.skew, "lines": len(page.lines)})
        self.recorder = None
        return "\n\f\n".join(pages)


def run(case, engine, output: Path):
    path = Path(case["path"])
    if sha(path) != case["input_sha256"]:
        raise RuntimeError("input changed after plan freeze")
    destination = output / case["id"]
    destination.mkdir()
    record = {**case, "started_utc": datetime.now(timezone.utc).isoformat()}
    capture = None
    begin = time.monotonic()
    try:
        signal.alarm(300)
        plain = engine.read(path, None)
        (destination / "plain.txt").write_text(plain, encoding="utf-8")
        capture = Recorder(destination / "tensors")
        instrumented = engine.read(path, capture)
        (destination / "text.txt").write_text(instrumented, encoding="utf-8")
        dump(destination / "details.json", engine.details)
        if instrumented != plain:
            raise RuntimeError("reference capture changed final text")
        record.update({
            "status": "ok", "text_sha256": sha(destination / "text.txt"),
            "output_bytes": len(instrumented.encode("utf-8")),
            "capture_matches_plain": True,
            "tensor_counts": dict(Counter(event["name"] for event in capture.events)),
        })
    except BaseException as error:
        record.update({"status": "error", "error_class": type(error).__name__, "error_message": str(error)})
        (destination / "failure.txt").write_text(traceback.format_exc(), encoding="utf-8")
        engine.recorder = None
    finally:
        signal.alarm(0)
        if capture is not None:
            capture.close()
        record["diagnostic_elapsed_seconds"] = time.monotonic() - begin
        dump(destination / "result.json", record)
    print(json.dumps({"id": case["id"], "status": record["status"], "output_bytes": record.get("output_bytes"),
                      "elapsed_diagnostic_seconds": record["diagnostic_elapsed_seconds"]}), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-only", action="store_true")
    parser.add_argument("--output", type=Path, default=EVIDENCE / "initial")
    args = parser.parse_args()
    args.output.mkdir()
    environment = verify_environment()
    dump(args.output / "environment.json", environment)
    (args.output / "opencv-build.txt").write_text(cv2.getBuildInformation())
    frozen = plan()
    dump(args.output / "plan.json", frozen)
    if args.plan_only:
        return
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("reference case exceeded300 seconds")))
    records = []
    engine = None
    active_math = None
    for case in frozen["cases"]:
        if active_math != case["math"]:
            engine = None
            engine = InstrumentedReference(case["math"])
            active_math = case["math"]
        records.append(run(case, engine, args.output))
    dump(args.output / "results.json", records)
    if any(record["status"] != "ok" for record in records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
