"""Locked-reference diagnostics only; this is not a production OCR path.

Freeze comparison rules before obtaining native predictions. Preserve every
attempt in an exclusive directory, including failed text and tensor comparisons.
The Rust binary is executed, never reimplemented or simulated here.
"""
from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import traceback

import numpy as np

ROOT = Path(__file__).resolve().parent
REFERENCE = ROOT / "initial"
PROTOCOL = ROOT / "parity-protocol.json"
EXACT_INPUTS = {
    "decoded", "upright", "orientation-input", "detection-input",
    "script-input", "text-input", "chinese-input",
    "math-classifier-input", "math-input",
}
BOXES = {"detection-boxes", "detection-boxes-unscaled"}
EXTRA_NATIVE = {"line-indices"}


def sha(path: Path) -> str:
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def dump(path: Path, value) -> None:
    with path.open("x", encoding="utf-8") as file:
        json.dump(value, file, ensure_ascii=False, indent=2, allow_nan=False)
        file.write("\n")


def freeze() -> None:
    plan = json.loads((REFERENCE / "plan.json").read_text())
    hashes = {}
    for case in plan["cases"]:
        directory = REFERENCE / case["id"]
        hashes[case["id"]] = {
            "text": sha(directory / "text.txt"),
            "manifest": sha(directory / "tensors/manifest.json"),
        }
    dump(PROTOCOL, {
        "schema_version": 1,
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "frozen_before_any_new_native_predictions": True,
        "purpose": "implementation parity, not OCR quality or attested performance",
        "selection_plan_sha256": sha(REFERENCE / "plan.json"),
        "reference_environment_sha256": sha(REFERENCE / "environment.json"),
        "reference_assets": hashes,
        "comparison_harness_sha256": sha(Path(__file__)),
        "settings": {"threads": 1, "cv_threads": 1, "batch_size": 64,
                     "intra_op_spinning": True, "inter_op_spinning": True},
        "rules": {
            "text": "exact UTF-8 bytes including whitespace and page separators",
            "capture": "native captured and uncaptured UTF-8 bytes must agree",
            "sequence": "all common tensor names in exact execution order, with identical counts",
            "extra_native_names": sorted(EXTRA_NATIVE),
            "shape_and_dtype": "exact; no reshape, transpose, sorting or dtype coercion",
            "integers_and_booleans": "exact values",
            "exact_preprocessing_tensors": sorted(EXACT_INPUTS),
            "other_float_tensors": {"atol": 1e-5, "rtol": 1e-5},
            "detector_box_tensors": {"atol": 1e-3, "rtol": 1e-5},
            "turn_skew": {"rotation": "exact", "skew_atol": 1e-12, "skew_rtol": 0},
            "nonfinite": "reject NaN and infinity in either arm",
            "failures": "preserve all raw outputs and every failed tensor; do not relax thresholds to obtain a pass",
        },
        "process_timeout_seconds": 420,
        "cpu_quota_attested": False,
        "timing_claim": "none; only real linked correctness diagnostics",
    })
    print(json.dumps({"protocol": str(PROTOCOL), "sha256": sha(PROTOCOL)}), flush=True)


def arrays(expected_file: Path, actual_file: Path, name: str) -> dict:
    expected = np.load(expected_file, allow_pickle=False, mmap_mode="r")
    actual = np.load(actual_file, allow_pickle=False, mmap_mode="r")
    result = {
        "name": name, "reference": expected_file.name, "native": actual_file.name,
        "reference_sha256": sha(expected_file), "native_sha256": sha(actual_file),
        "reference_shape": list(expected.shape), "native_shape": list(actual.shape),
        "reference_dtype": expected.dtype.str, "native_dtype": actual.dtype.str,
    }
    if actual.shape != expected.shape or actual.dtype != expected.dtype:
        return {**result, "pass": False, "reason": "shape_or_dtype"}
    if not np.isfinite(actual).all() or not np.isfinite(expected).all():
        return {**result, "pass": False, "reason": "nonfinite"}
    exact = name in EXACT_INPUTS or expected.dtype.kind in "biu"
    if exact:
        okay = actual == expected
        result["mode"] = "exact"
    else:
        absolute, relative = (1e-3, 1e-5) if name in BOXES else (1e-5, 1e-5)
        if name == "turn-skew":
            absolute, relative = 1e-12, 0.0
        okay = np.isclose(actual, expected, atol=absolute, rtol=relative, equal_nan=False)
        if name == "turn-skew":
            okay[0] = actual[0] == expected[0]
        result.update(mode="numeric", atol=absolute, rtol=relative)
    failures = int(np.count_nonzero(~okay))
    result.update({"pass": failures == 0, "elements": int(expected.size),
                   "unequal_values": int(np.count_nonzero(actual != expected)),
                   "outside_tolerance": failures})
    if expected.size:
        delta = np.abs(actual.astype(np.float64) - expected.astype(np.float64))
        result["max_absolute_error"] = float(delta.max())
        nonzero = expected != 0
        result["max_relative_error_nonzero_reference"] = (
            float((delta[nonzero] / np.abs(expected[nonzero])).max())
            if np.any(nonzero) else 0.0
        )
    if failures:
        first = np.flatnonzero(~okay)[:16]
        result["first_failed_flat_indices"] = first.tolist()
        result["reference_values"] = expected.ravel()[first].tolist()
        result["native_values"] = actual.ravel()[first].tolist()
    return result


def compare(case: dict, directory: Path, protocol: dict) -> dict:
    expected = REFERENCE / case["id"]
    locked = protocol["reference_assets"][case["id"]]
    if sha(expected / "text.txt") != locked["text"]:
        raise RuntimeError("reference text changed after protocol freeze")
    if sha(expected / "tensors/manifest.json") != locked["manifest"]:
        raise RuntimeError("reference tensor manifest changed after protocol freeze")
    events = json.loads((expected / "tensors/manifest.json").read_text())["events"]
    for event in events:
        if sha(expected / "tensors" / event["file"]) != event["sha256"]:
            raise RuntimeError("reference tensor changed after reference capture")
    native = sorted((directory / "tensors").glob("*.npy"))
    native_events = [(file.name.split("-", 1)[1][:-4], file) for file in native]
    filtered = [(name, file) for name, file in native_events if name not in EXTRA_NATIVE]
    expected_names = [event["name"] for event in events]
    native_names = [name for name, _ in filtered]
    text = (directory / "text.txt").read_bytes()
    plain = (directory / "plain.txt").read_bytes()
    reference_text = (expected / "text.txt").read_bytes()
    result = {
        "text_equal": text == reference_text,
        "plain_text_equal": plain == reference_text,
        "capture_matches_plain": text == plain,
        "reference_text_sha256": locked["text"],
        "native_text_sha256": sha(directory / "text.txt"),
        "native_plain_sha256": sha(directory / "plain.txt"),
        "native_output_bytes": len(text), "reference_output_bytes": len(reference_text),
        "tensor_sequence_equal": expected_names == native_names,
        "reference_tensor_counts": dict(Counter(expected_names)),
        "native_tensor_counts": dict(Counter(name for name, _ in native_events)),
    }
    comparisons = []
    # Once the sequence diverges, compare each semantic name by occurrence index
    # for diagnosis, while retaining the sequence gate as an unconditional fail.
    occurrence = Counter()
    by_name = {}
    for name, path in filtered:
        by_name.setdefault(name, []).append(path)
    for event in events:
        name = event["name"]
        index = occurrence[name]
        occurrence[name] += 1
        matches = by_name.get(name, [])
        if index >= len(matches):
            comparisons.append({"name": name, "occurrence": index, "pass": False,
                                "reason": "missing_native_event"})
            continue
        comparisons.append({**arrays(expected / "tensors" / event["file"], matches[index], name),
                            "occurrence": index})
    dump(directory / "tensor-comparison.json", comparisons)
    result["failed_tensors"] = sum(not item["pass"] for item in comparisons)
    result["compared_tensors"] = len(comparisons)
    result["pass"] = all(result[key] for key in ["text_equal", "plain_text_equal",
        "capture_matches_plain", "tensor_sequence_equal"]) and result["failed_tensors"] == 0
    return result


def run(binary: Path, models: Path, output: Path) -> None:
    protocol = json.loads(PROTOCOL.read_text())
    for name, expected in (("plan.json", protocol["selection_plan_sha256"]),
                           ("environment.json", protocol["reference_environment_sha256"])):
        if sha(REFERENCE / name) != expected:
            raise RuntimeError("reference selection or environment changed after freeze")
    if sha(Path(__file__)) != protocol["comparison_harness_sha256"]:
        raise RuntimeError("comparison harness changed after protocol freeze")
    output.mkdir()
    environment = os.environ.copy()
    for name in ["OMP_NUM_THREADS", "OMP_THREAD_LIMIT", "OPENBLAS_NUM_THREADS",
                 "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS", "VECLIB_MAXIMUM_THREADS"]:
        environment[name] = "1"
    dump(output / "environment.json", {
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "binary": str(binary), "binary_sha256": sha(binary),
        "protocol_sha256": sha(PROTOCOL), "model_directory": str(models),
        "cpu_quota_attested": False, "purpose": protocol["purpose"],
        "thread_environment": {key: value for key, value in environment.items()
                               if key in ["OMP_NUM_THREADS", "OMP_THREAD_LIMIT",
                                          "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS",
                                          "NUMEXPR_NUM_THREADS", "VECLIB_MAXIMUM_THREADS"]},
    })
    results = []
    for case in json.loads((REFERENCE / "plan.json").read_text())["cases"]:
        directory = output / case["id"]
        directory.mkdir()
        result = {"id": case["id"], "started_utc": datetime.now(timezone.utc).isoformat(),
                  "input_sha256": case["input_sha256"], "math": case["math"]}
        try:
            if sha(Path(case["path"])) != case["input_sha256"]:
                raise RuntimeError("input changed after selection freeze")
            base = [str(binary), case["path"], "--model-dir", str(models),
                    "--threads", "1", "--cv-threads", "1", "--batch-size", "64",
                    "--intra-op-spinning", "true", "--inter-op-spinning", "true",
                    "--timeout-seconds", "300"]
            if case["math"]:
                base.append("--math")
            commands = []
            for label in ["plain", "captured"]:
                text_file = "plain.txt" if label == "plain" else "text.txt"
                command = base + ["--output", str(directory / text_file),
                                  "--stats", str(directory / f"{label}.jsonl")]
                if label == "captured":
                    command += ["--trace-directory", str(directory / "tensors")]
                commands.append(command)
            dump(directory / "commands.json", commands)
            exits = []
            for label, command in zip(["plain", "captured"], commands, strict=True):
                with (directory / f"{label}.stdout").open("xb") as stdout, \
                     (directory / f"{label}.stderr").open("xb") as stderr:
                    completed = subprocess.run(command, env=environment, stdout=stdout,
                        stderr=stderr, timeout=protocol["process_timeout_seconds"], check=False)
                exits.append(completed.returncode)
            result["exit_codes"] = exits
            if any(code != 0 for code in exits):
                result.update({"pass": False, "reason": "native_process_failed"})
            else:
                result.update(compare(case, directory, protocol))
        except Exception:
            result.update({"pass": False, "error": traceback.format_exc()})
        dump(directory / "result.json", result)
        results.append(result)
        print(json.dumps({key: result.get(key) for key in
            ["id", "pass", "text_equal", "failed_tensors", "reason", "error"]}), flush=True)
    dump(output / "results.json", results)
    if any(not item["pass"] for item in results):
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("freeze")
    execute = commands.add_parser("run")
    execute.add_argument("binary", type=Path)
    execute.add_argument("models", type=Path)
    execute.add_argument("output", type=Path)
    arguments = parser.parse_args()
    if arguments.command == "freeze":
        freeze()
    else:
        run(arguments.binary.resolve(), arguments.models.resolve(), arguments.output.resolve())
