"""Reference-only evidence harness for Indra OCR 1.1.2. Never imported by the Rust service.

Runs the unmodified, hash-locked upstream Python implementation. Wrappers record
real arguments/results; they do not replace OCR algorithms. Timings here are
diagnostic only. Paths are explicit arguments so the harness is relocatable:

  reference.py --repo REPO --models MODELS --upstream UPSTREAM --output OUT \
      [--assets DIR] [--corpus DIR] [--plan-only]

UPSTREAM holds the indra_ocr/*.py files of commit
0e96b93c6810f17cf8352acbb4487cc9e45b66d5 (tag v1.1.2); ASSETS the two Hub example
pages of revision bd02167f; CORPUS the frozen evaluation inputs (prepared/...).
Cases whose inputs are absent are recorded as skipped, never silently dropped.
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
import sys
import time
import traceback

REVISION = "0e96b93c6810f17cf8352acbb4487cc9e45b66d5"
PACKAGE_VERSION = "1.1.2"
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

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures"

# Frozen inputs: (id, where, relative path, sha256 or None when computed at plan time, purpose, math)
CASES = [
    ("smoke", "repo", "backend/tests/data/smoke.png",
     "b6f8717e9db060d7c34c05f39f090b5d211a999edc2715149367967c30c87d3a", "short PNG decoding and exact integration text", False),
    ("two-pages", "repo", "backend/tests/data/two-pages.tiff",
     "303ebb45259b96fecc0b200edf8e0a68ed50e2b43f441bceef8694e43699c329", "classic TIFF frame decoding and page boundary", False),
    ("jpeg", "repo", "benchmarks/results/synthetic-fixtures/jpeg_200.jpg",
     "e7abb6f4817f74da24eb29713dd8f1ef190f8a1bb134ab419e12b45ce7ae372d", "JPEG decoder parity; synthetic integration fixture", False),
    ("skew", "repo", "benchmarks/results/synthetic-fixtures/skew_200.png", None, "deskew path; synthetic integration fixture", False),
    ("bhl-deu-0018", "corpus", "prepared/bhl/bhl-deu-0018.png",
     "f6701d55eafc65643763b09c9a80e5c74238484f81c28d2fccba4dedfafa68fa", "prespecified representative: Latn, historical_natural_science, full_page", False),
    ("eurlex-ai-act-el-p0002", "corpus", "prepared/eurlex-ai-act-el/page-0002.png",
     "7d51bb17cba9bb6d00d5e243f7fe999e5b22a566f803db81a8c67517b3e5a224", "prespecified representative: Grek, modern_legal, full_page", False),
    ("bongabdo-img_20230101_0016", "corpus", "prepared/bongabdo/bongabdo-img_20230101_0016.png",
     "444ba6067d4847bce820fa0d59e6848f7a7c9a01d80db9c50ff5d5f20f1c19ab", "prespecified representative: Beng, literary_prose, full_page", False),
    ("sankaracarya1925-03-body", "corpus", "prepared/devanagari-body/sankaracarya1925-03-body.png",
     "f5d7f66e98c2a57225a28b15cfa2e7bd06c753a0b4e9dc215ac6c24344447401", "prespecified representative: Deva, historical_philosophy, oracle_crop", False),
    ("jingbao-1930-500", "corpus", "prepared/chinese/jingbao-1930-500.png",
     "79800ce399a19933e9cee9dea29fa641b15334fe9f13c1ee024e77785b2a8318", "prespecified representative: Hant, historical_newspaper, oracle_crop", False),
    ("AMSMB_1-3-100", "corpus", "prepared/amsmb/AMSMB_1-3-100.png",
     "23557f8642928a8e91a1b721c987ef91b08effe699070822dc06a4d0f51e6733", "prespecified representative: Latn, historical_notarial_charter, full_page", False),
    ("lectaurep-riant-01", "corpus", "prepared/lectaurep/lectaurep-riant-01.png",
     "7153a8469459230012c1e7560a8f85fd812b3b4bf4060a6eb36acc0f6518b1c0", "prespecified representative: Latn, historical_notarial_register, full_page", False),
    ("rotated-greek", "generated", "diagnostics-rotated-greek.png", None,
     "one clockwise quarter-turn of the frozen Greek legal page, using OpenCV 5.0.0.93", False),
    ("math-off", "repo", "runtime/crates/indra-engine/tests/data/math.png",
     "5482d2f8acdedda736c020bfde851a5358730dad168e2ad7920a38f21608ab61", "synthetic LaTeX equation page at 200 dpi; optional math disabled", False),
    ("math-on", "repo", "runtime/crates/indra-engine/tests/data/math.png",
     "5482d2f8acdedda736c020bfde851a5358730dad168e2ad7920a38f21608ab61", "same synthetic LaTeX equation page; classifier and actual autoregressive math path", True),
    ("hf-handwriting", "assets", "handwriting_letter.png",
     "bed634b6df330b040cd52d8d9104433f54e458c16ed87607c406c009b98ecd25", "Hub example: handwritten cursive letter; base recogniser with LM rescoring", False),
    ("hf-korean", "assets", "korean_letter.png",
     "973d6e2aca838ba5da72ab691bd8d0d9820e907002ca398eb884a1c87a8efb29", "Hub example: scanned Korean letter; CJK routing, Hangul jamo composition, dual reads", False),
    ("synthetic-rtl", "fixtures", "synthetic-rtl.png",
     "70cdb1f220a3ee79d603f8223d77b4ce9549190a10d5b3da76c24d50b8114c93", "CoreText-rendered Arabic and Hebrew paragraphs with Latin and digits; RTL recogniser, page direction, flip, mirrored order features", False),
    ("synthetic-world", "fixtures", "synthetic-world.png",
     "15f3412c16f90d714f2232189112d4b8ac0fe0ceaba399d280da1cad828d59be", "CoreText-rendered Thai, Devanagari and Bengali paragraphs; world recogniser with lm_world rescoring", False),
    ("synthetic-vertical-cjk", "fixtures", "synthetic-vertical-cjk.png",
     "9ca05b26a33d255c6f62f682109ac22210ad56233717550c3b0c3a97d0260a63", "CoreText-rendered vertical Traditional Chinese columns; orientation class 4, CJK columns, XY-cut blocks", False),
]


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def dump(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def verify_environment(args) -> dict:
    model_lock = args.repo / "backend/models.lock.json"
    lock = json.loads(model_lock.read_text())
    if lock["package_version"] != PACKAGE_VERSION or lock["code_revision"] != REVISION:
        raise RuntimeError("model lock does not bind the expected package and code revision")
    for artifact in lock["files"]:
        file = args.models / artifact["name"]
        if file.stat().st_size != artifact["size"] or sha(file) != artifact["sha256"]:
            raise RuntimeError(f"model artifact verification failed: {artifact['name']}")
    package = Path(pipeline.__file__).parent
    source_hashes = {}
    upstream_files = sorted(args.upstream.glob("*.py"))
    if len(upstream_files) != 18:
        raise RuntimeError("upstream directory must hold the 18 indra_ocr modules of the commit")
    for file in sorted(package.glob("*.py")):
        source = args.upstream / file.name
        if not source.is_file() or file.read_bytes() != source.read_bytes():
            raise RuntimeError(f"installed reference source differs from commit: {file.name}")
        source_hashes[file.name] = sha(file)
    versions = {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()}
    if versions.get("indra-ocr") != PACKAGE_VERSION or np.__version__ != "2.5.4" or ort.__version__ != "1.31.0":
        raise RuntimeError("unexpected locked numerical/reference package version")
    if not cv2.__version__.startswith("5.0.0"):
        raise RuntimeError("unexpected OpenCV version")
    cv2.setNumThreads(1)
    affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None
    return {
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "purpose": "fresh implementation parity for Indra OCR 1.1.2; not a quality estimate or resource-attested performance measurement",
        "python": sys.version,
        "python_executable_sha256": sha(Path(sys.executable).resolve()),
        "packages": versions,
        "requirements_lock_sha256": sha(args.repo / "backend/requirements.lock"),
        "harness_sha256": sha(Path(__file__)),
        "model_lock_sha256": sha(model_lock),
        "model_snapshot": lock,
        "upstream_commit": REVISION,
        "installed_source_hashes": source_hashes,
        "machine": platform.machine(),
        "system": platform.system(),
        "kernel": platform.release(),
        "affinity": affinity,
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
            "lm": True, "route_sure": pipeline.ROUTE_SURE, "orient_min": pipeline.ORIENT_MIN,
            "deskew_max": pipeline.DESKEW_MAX, "page_separator": "\n\f\n",
            "reference_option_override": "A SessionOptions factory only sets memory_pattern=False and explicit spinning flags before the unchanged upstream constructor; Python otherwise leaves memory patterns enabled by default.",
        },
    }


def plan(args) -> dict:
    cases = []
    skipped = []
    roots = {"repo": args.repo, "assets": args.assets, "corpus": args.corpus, "fixtures": FIXTURES, "generated": args.output}
    for identifier, where, relative, expected, purpose, math in CASES:
        root = roots[where]
        if root is None:
            skipped.append({"id": identifier, "reason": f"no --{where} directory supplied"})
            continue
        path = root / relative
        if where == "generated":
            source = args.corpus / "prepared/eurlex-ai-act-el/page-0002.png" if args.corpus else None
            if source is None or not source.is_file():
                skipped.append({"id": identifier, "reason": "the frozen Greek page is not available to rotate"})
                continue
            gray = cv2.imread(str(source), cv2.IMREAD_GRAYSCALE)
            if gray is None or not cv2.imwrite(str(path), cv2.rotate(gray, cv2.ROTATE_90_CLOCKWISE)):
                raise RuntimeError("rotation fixture generation failed")
        if not path.is_file():
            skipped.append({"id": identifier, "reason": f"missing input {path}"})
            continue
        digest = sha(path)
        if expected is not None and digest != expected:
            raise RuntimeError(f"frozen input mismatch: {identifier}")
        cases.append({"id": identifier, "path": str(path), "input_sha256": digest,
                      "input_bytes": path.stat().st_size, "math": math, "purpose": purpose})
    return {
        "created_before_inference": True,
        "selection_policy": "The fourteen 2026-10-18 implementation cases (transport fixtures, seven script/domain representatives, rotation derivative, synthetic math pair) plus the two Hub example pages and three CoreText-rendered pages that exercise the 1.1 paths (RTL, world scripts, vertical CJK); selected before inspecting any 1.1 model outputs.",
        "math_source_sha256": sha(args.repo / "runtime/crates/indra-engine/tests/data/math.tex"),
        "fixture_renderers": {name: sha(FIXTURES / name) for name in ["render-pages.swift", "render-vertical.swift"]},
        "cases": cases,
        "skipped": skipped,
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
    "rec_rtl": {"x": "rtl-input", "widths": "line-widths"},
    "rec_world": {"x": "world-input", "widths": "line-widths"},
    "order": {"feats": "order-features"},
    "mathcls": {"x": "math-classifier-input", "widths": "line-widths"},
    "math_enc": {"x": "math-input", "widths": "math-widths"},
    "math_init": {}, "math_step": {},
}
OUTPUT_NAMES = {
    "orient": ["orientation-output"], "det": ["detection-output"],
    "script": ["script-probabilities"],
    "rec": ["text-ids", "text-confidences", "ctc-lengths", "text-blank", "text-topv", "text-topi"],
    "rec_cjk": ["chinese-ids", "chinese-confidences", "ctc-lengths", "chinese-blank", "chinese-topv", "chinese-topi"],
    "rec_rtl": ["rtl-ids", "rtl-confidences", "ctc-lengths"],
    "rec_world": ["world-ids", "world-confidences", "ctc-lengths", "world-blank", "world-topv", "world-topi"],
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
    def __init__(self, models: Path, math: bool):
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
                models, threads=1, rec_int8=False, adaptive=True,
                table_heads=False, math=math, providers=["CPUExecutionProvider"],
            )
        finally:
            ort.SessionOptions = original
        engine = self.engine
        if tuple(engine.script_groups) != ("base", "cjk", "rtl", "world") or set(engine.groups) != {"cjk", "rtl", "world"}:
            raise RuntimeError("script groups differ from the locked model metadata")
        if set(engine.lms) != {id(engine.rec), id(engine.groups["cjk"][0]), id(engine.groups["world"][0])}:
            raise RuntimeError("the base, CJK and world language models must be loaded")

        def wrap(session, label):
            wrapped = SessionRecorder(session, label, self)
            # The pipeline keys language models by session identity and selects
            # the tall window for the base session by identity: re-key after wrapping.
            if id(session) in engine.lms:
                engine.lms[id(wrapped)] = engine.lms.pop(id(session))
            return wrapped

        for name in ("orient", "det", "script", "rec", "order"):
            setattr(engine, name, wrap(getattr(engine, name), name))
        for group, (session, vocab) in list(engine.groups.items()):
            engine.groups[group] = (wrap(session, f"rec_{group}"), vocab)
        engine.lm = engine.lms[id(engine.rec)][0]
        if math:
            for name, label in (("cls", "mathcls"), ("enc", "math_enc"), ("init", "math_init"), ("step", "math_step")):
                setattr(engine.math, name, SessionRecorder(getattr(engine.math, name), label, self))
            original_read = engine.math.read
            math_code = original_read.__func__.__code__
            step_end = next(
                number for number, line in enumerate(Path(math_code.co_filename).read_text().splitlines(), 1)
                if line.strip() == "if done.all():"
            )

            def math_read(*args, **kwargs):
                if self.recorder is None:
                    return original_read(*args, **kwargs)

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

            engine.math.read = math_read
        original_turn = engine._turn

        def turn(*args, **kwargs):
            result = original_turn(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("turn-skew", np.asarray([result[1], result[2]], dtype=np.float64))
                self.recorder.save("upright", result[0])
            return result

        engine._turn = turn
        original_at = engine._detect_at

        def detect_at(*args, **kwargs):
            result = original_at(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("detection-boxes-unscaled", result[0][:, :4])
            return result

        engine._detect_at = detect_at
        original_detect = engine.detect

        def detect(*args, **kwargs):
            result = original_detect(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("detection-boxes", result[:, :4])
            return result

        engine.detect = detect
        original_darkness = pipeline.line_darkness

        def line_darkness(*args, **kwargs):
            result = original_darkness(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("line-dark", np.asarray(result, dtype=bool))
            return result

        pipeline.line_darkness = line_darkness
        original_groups = engine.line_groups
        order_of_groups = list(engine.script_groups)

        def line_groups(*args, **kwargs):
            names, sure = original_groups(*args, **kwargs)
            if self.recorder is not None:
                self.recorder.save("line-groups", np.asarray([order_of_groups.index(name) for name in names], dtype=np.int64))
            self.details.append({"groups": names, "sure": sure})
            return names, sure

        engine.line_groups = line_groups
        original_order = engine.reading_order

        def reading_order(boxes, texts, wh):
            result = original_order(boxes, texts, wh)
            if self.recorder is not None and len(boxes) >= 2:
                self.recorder.save("order-permutation", np.asarray(result[0], dtype=np.int64))
            self.details.append({"split_boxes": boxes.tolist(), "split_texts": texts, "order": result[0]})
            return result

        engine.reading_order = reading_order

    def read(self, path: Path, recorder: Recorder | None):
        self.recorder = recorder
        self.details = []
        pages = []
        for gray in pipeline.read_images(path):
            if recorder is not None:
                recorder.save("decoded", gray)
            page = self.engine(gray)
            pages.append(page.text)
            self.details.append({"rotation": page.rotation, "skew": page.skew, "vertical": page.vertical,
                                 "page_rtl": self.engine._page_rtl, "lines": len(page.lines)})
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
        signal.alarm(600)
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
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--upstream", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--plan-only", action="store_true")
    args = parser.parse_args()
    args.output.mkdir()
    environment = verify_environment(args)
    dump(args.output / "environment.json", environment)
    (args.output / "opencv-build.txt").write_text(cv2.getBuildInformation())
    frozen = plan(args)
    dump(args.output / "plan.json", frozen)
    if args.plan_only:
        return
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("reference case exceeded 600 seconds")))
    records = []
    engine = None
    active_math = None
    for case in frozen["cases"]:
        if active_math != case["math"]:
            engine = None
            engine = InstrumentedReference(args.models, case["math"])
            active_math = case["math"]
        records.append(run(case, engine, args.output))
    dump(args.output / "results.json", records)
    if any(record["status"] != "ok" for record in records):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
