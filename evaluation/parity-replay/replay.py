"""Relocatable replay of the preserved fourteen-case reference diagnostics.

Reference/audit tooling only. Original selection, algorithms, thresholds and
recorded harness bytes remain untouched. Fresh tensor arrays are new evidence,
never claimed as recovery of deleted historical raw arrays.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import sys

REFERENCE_IMAGE = "ghcr.io/somehowchris/tika-indra-backend@sha256:dead953f64a4119e3f95e1bed3df17ddcbdea7573adbe3e07d528c31778f91bd"
REVISION = "3d773aeedc5d79d1d0a2a04007a9075cd71a59f4"
THREADS = ("OMP_NUM_THREADS", "OMP_THREAD_LIMIT", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS", "VECLIB_MAXIMUM_THREADS")
FROZEN = Path("runtime/crates/indra-engine/tests/evidence/2026-10-18-parity")


def require(ok, why):
    if not ok:
        raise ValueError(why)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2, allow_nan=False)
        stream.write("\n")


def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def locate(case, repo, corpus):
    original = case["path"]
    if "/repo/" in original:
        return repo / original.split("/repo/", 1)[1]
    if "/corpus-data/" in original:
        return corpus / original.split("/corpus-data/", 1)[1]
    if case["id"] in {"math-off", "math-on"}:
        return repo / "runtime/crates/indra-engine/tests/data/math.png"
    if case["id"] == "rotated-greek":
        return None
    raise ValueError("unrecognized frozen case path")


def verify_input(path, case):
    require(path.is_file() and not path.is_symlink(), "input must be a regular non-symlink file")
    require(path.stat().st_size == case["input_bytes"] and sha(path) == case["input_sha256"], "frozen input byte identity differs: " + case["id"])


def stage(args):
    verify_preserved(args.repo)
    args.inputs.mkdir()
    frozen = read(args.repo / FROZEN / "selection-plan.json")
    records = []
    for case in frozen["cases"]:
        source = locate(case, args.repo, args.corpus)
        suffix = Path(case["path"]).suffix
        target = args.inputs / (case["id"] + suffix)
        if source is not None:
            verify_input(source, case)
            shutil.copyfile(source, target)
            verify_input(target, case)
        records.append({"id": case["id"], "original_path": case["path"], "container_path": "/inputs/" + target.name,
                        "host_source": str(source) if source else None, "input_sha256": case["input_sha256"],
                        "input_bytes": case["input_bytes"], "status": "verified" if source else "rotation_pending"})
    write(args.inputs / "path-map.json", {"schema_version": 1, "selection_sha256": sha(args.repo / FROZEN / "selection-plan.json"), "cases": records})
    print(json.dumps({"staged_cases": len(records) - 1, "rotation_pending": 1, "predictions": 0}))


def verify_preserved(repo):
    root = repo / FROZEN
    require(sha(root / "protocol.json") == "903a8a82a8ad9957cbe614dc4395e40db9065da200fe153a10aed91158f4ca68", "original protocol changed")
    protocol = read(root / "protocol.json")
    environment = read(root / "reference-environment.json")
    require(sha(root / "selection-plan.json") == protocol["selection_plan_sha256"], "original selection changed")
    require(sha(root / "reference-environment.json") == protocol["reference_environment_sha256"], "original environment changed")
    require(sha(root / "recorded-harness/reference.py") == environment["harness_sha256"], "original reference harness changed")
    require(sha(root / "recorded-harness/native_parity.py") == protocol["comparison_harness_sha256"], "original comparison harness changed")


def numerical_environment(args):
    for key in THREADS:
        os.environ[key] = "1"
    os.environ["HF_HUB_OFFLINE"] = "1"
    import cv2
    import numpy as np
    import onnxruntime as ort
    import indra_ocr.pipeline as pipeline
    from importlib.metadata import version
    require(np.__version__ == "2.5.4" and ort.__version__ == "1.31.0" and version("indra-ocr") == "0.3.2" and
            version("opencv-python-headless") == "5.0.0.93", "locked reference package version differs")
    original = read(args.repo / FROZEN / "reference-environment.json")
    package = Path(pipeline.__file__).parent
    expected = original["installed_source_hashes"]
    actual_paths = {str(p.relative_to(package)) for p in package.rglob("*.py")}
    require(actual_paths == set(expected), "installed upstream source inventory differs")
    for relative, expected_sha in expected.items():
        installed = package / relative
        upstream = args.upstream / "indra_ocr" / relative
        require(sha(installed) == expected_sha and sha(upstream) == expected_sha and installed.read_bytes() == upstream.read_bytes(),
                "installed/upstream source differs: " + relative)
    require(sha(args.repo / "backend/requirements.lock") == original["requirements_lock_sha256"] and
            sha(args.repo / "backend/models.lock.json") == original["model_lock_sha256"], "locked package/model metadata differs")
    lock = read(args.repo / "backend/models.lock.json")
    for asset in lock["files"]:
        path = args.models / asset["name"]
        require(path.stat().st_size == asset["size"] and sha(path) == asset["sha256"], "locked model artifact differs: " + asset["name"])
    cv2.setNumThreads(1)
    return {"schema_version": 1, "created_utc": datetime.now(timezone.utc).isoformat(), "python": sys.version,
            "python_executable_sha256": sha(Path(sys.executable).resolve()), "platform": platform.platform(),
            "packages": {d.metadata["Name"]: d.version for d in importlib.metadata.distributions()},
            "installed_source_hashes": expected, "upstream_commit": REVISION, "reference_image": REFERENCE_IMAGE,
            "model_lock_sha256": sha(args.repo / "backend/models.lock.json"), "requirements_lock_sha256": sha(args.repo / "backend/requirements.lock"),
            "thread_environment": {k: os.environ[k] for k in THREADS}, "cpu_quota_attested": True,
            "scope": "new implementation diagnostic; no quality or performance claim; interpreter identity differs from historical host and is recorded explicitly"}


def snapshot(args, label):
    group = Path("/sys/fs/cgroup")
    require((group / "cpu.max").read_text().split() == ["100000", "100000"], "one actual CFS CPU required")
    require((group / "memory.max").read_text().strip() == "3221225472" and (group / "memory.swap.max").read_text().strip() == "0", "3GiB and zero swap required")
    destination = args.output / label
    destination.mkdir()
    for name in ("cpu.max", "cpu.stat", "cpuset.cpus.effective", "memory.max", "memory.swap.max", "memory.current", "memory.peak", "memory.events", "pids.max"):
        (destination / name).write_bytes((group / name).read_bytes())
    (destination / "process-status").write_bytes(Path("/proc/self/status").read_bytes())


def prepare(args):
    verify_preserved(args.repo)
    args.output.mkdir()
    environment = numerical_environment(args)
    frozen = read(args.repo / FROZEN / "selection-plan.json")
    mapping = read(args.inputs / "path-map.json")
    require(mapping["selection_sha256"] == sha(args.repo / FROZEN / "selection-plan.json"), "path map differs from original freeze")
    import cv2
    greek = next(c for c in mapping["cases"] if c["id"] == "eurlex-ai-act-el-p0002")
    rotated = next(c for c in mapping["cases"] if c["id"] == "rotated-greek")
    source = cv2.imread(greek["container_path"], cv2.IMREAD_GRAYSCALE)
    require(source is not None and cv2.imwrite(rotated["container_path"], cv2.rotate(source, cv2.ROTATE_90_CLOCKWISE)), "Greek rotation failed")
    by_id = {c["id"]: c for c in mapping["cases"]}
    relocated = {**frozen, "cases": [{**case, "path": by_id[case["id"]]["container_path"]} for case in frozen["cases"]]}
    for case in relocated["cases"]:
        verify_input(Path(case["path"]), case)
    write(args.output / "environment.json", environment)
    write(args.output / "selection-plan.json", relocated)
    original = args.repo / FROZEN
    write(args.output / "amendment.json", {
        "schema_version": 1, "created_utc": datetime.now(timezone.utc).isoformat(), "status": "prospective_before_fresh_reference_and_native_predictions",
        "original_selection_sha256": sha(original / "selection-plan.json"), "original_protocol_sha256": sha(original / "protocol.json"),
        "original_environment_sha256": sha(original / "reference-environment.json"), "original_reference_harness_sha256": sha(original / "recorded-harness/reference.py"),
        "original_comparison_harness_sha256": sha(original / "recorded-harness/native_parity.py"), "adapter_sha256": sha(Path(__file__)),
        "relocated_selection_sha256": sha(args.output / "selection-plan.json"), "fresh_environment_sha256": sha(args.output / "environment.json"),
        "native_binary_sha256": sha(args.binary), "path_map": mapping,
        "unchanged": ["all14 case IDs/input hashes/byte counts/math flags/order", "all model artifacts", "upstream source and numerical package versions", "original instrumenting hooks and comparison functions", "exact preprocessing/integer/shape/sequence rules and all numeric tolerances"],
        "changes": ["absolute filesystem paths only", "fresh immutable-image interpreter/environment identity", "fresh reference arrays will be generated and hash-frozen before native predictions, because original raw arrays were deleted"],
        "reference_identity_policy": "Report whether newly generated text/tensor manifests equal historical hashes, but never call new arrays recovered original bytes. Native comparison uses new source-verified captures under unchanged rules; preserve every mismatch.",
        "performance_claim": "none; correctness diagnostics only"})
    snapshot(args, "prepare-resources")
    print(json.dumps({"prepared_cases": 14, "predictions": 0, "amendment_sha256": sha(args.output / "amendment.json")}))


def admission(args):
    verify_preserved(args.repo)
    amended = read(args.output / "amendment.json")
    require(amended["adapter_sha256"] == sha(Path(__file__)) and amended["relocated_selection_sha256"] == sha(args.output / "selection-plan.json") and
            amended["fresh_environment_sha256"] == sha(args.output / "environment.json") and amended["native_binary_sha256"] == sha(args.binary), "frozen replay source/input/environment/binary identity changed")
    for case in read(args.output / "selection-plan.json")["cases"]:
        verify_input(Path(case["path"]), case)
    return amended


def reference(args):
    admission(args)
    numerical_environment(args)
    module = load_module(args.repo / FROZEN / "recorded-harness/reference.py", "preserved_reference")
    module.MODELS = args.models
    snapshot(args, "reference-before")
    output = args.output / "reference"
    output.mkdir()
    plan = read(args.output / "selection-plan.json")
    write(output / "plan.json", plan)
    shutil.copyfile(args.output / "environment.json", output / "environment.json")
    signal.signal(signal.SIGALRM, lambda *_: (_ for _ in ()).throw(TimeoutError("reference case exceeded 300 seconds")))
    engine = None
    active_math = None
    results = []
    for case in plan["cases"]:
        if active_math != case["math"]:
            engine = None
            engine = module.InstrumentedReference(case["math"])
            active_math = case["math"]
        results.append(module.run(case, engine, output))
    write(output / "results.json", results)
    snapshot(args, "reference-after")
    require(all(r["status"] == "ok" for r in results), "reference failures retained; native comparison is not admitted")
    original = read(args.repo / FROZEN / "protocol.json")
    assets = {c["id"]: {"text": sha(output / c["id"] / "text.txt"), "manifest": sha(output / c["id"] / "tensors/manifest.json")} for c in plan["cases"]}
    write(args.output / "historical-reference-identities.json", {key: {"text_matches_historical": assets[key]["text"] == old["text"], "manifest_matches_historical": assets[key]["manifest"] == old["manifest"]} for key, old in original["reference_assets"].items()})
    protocol = {**original, "created_utc": datetime.now(timezone.utc).isoformat(), "selection_plan_sha256": sha(output / "plan.json"),
                "reference_environment_sha256": sha(output / "environment.json"), "reference_assets": assets,
                "amendment_sha256": sha(args.output / "amendment.json"), "reference_policy": "fresh source-verified arrays; original raw arrays not recovered",
                "cpu_quota_attested": True, "frozen_before_any_new_native_predictions": True}
    write(args.output / "comparison-protocol.json", protocol)
    print(json.dumps({"reference_cases": len(results), "native_predictions": 0, "comparison_protocol_sha256": sha(args.output / "comparison-protocol.json")}))


def native(args):
    admission(args)
    for key in THREADS:
        os.environ[key] = "1"
    module = load_module(args.repo / FROZEN / "recorded-harness/native_parity.py", "preserved_comparison")
    module.REFERENCE = args.output / "reference"
    module.PROTOCOL = args.output / "comparison-protocol.json"
    snapshot(args, "native-before")
    try:
        module.run(args.binary, args.models, args.output / "native")
    finally:
        snapshot(args, "native-after")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("stage", "prepare", "reference", "native"))
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, required=True)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--upstream", type=Path)
    parser.add_argument("--models", type=Path, default=Path("/models"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--binary", type=Path, default=Path("/native-ocr"))
    args = parser.parse_args()
    if args.action == "stage":
        require(args.corpus is not None, "stage requires --corpus")
        stage(args)
    else:
        require(args.output is not None and args.upstream is not None, "replay requires --output and --upstream")
        globals()[args.action](args)


if __name__ == "__main__":
    main()
