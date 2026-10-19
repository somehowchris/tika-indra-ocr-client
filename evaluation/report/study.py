"""Fail-closed admission of the Rust evaluator's combined scientific artifacts.

This is report validation, never an OCR runner or an alternative scorer. The
source revision must be selected by the reviewer, not trusted from downloaded
JSON. SHA inventories detect changes, not authenticity: upstream Cosign and OCI
verification receipts remain a separate required release gate.
"""
from __future__ import annotations

import csv
import hashlib
import io
import itertools
import json
import math
from pathlib import Path, PurePosixPath
import re

ROOT = Path(__file__).resolve().parents[2]
VARIANTS = (
    "tesseract-fast-psm3", "tesseract-best-psm3", "tesseract-best-psm6",
    "tesseract-best-psm11", "python-text-b64", "python-math-b64",
    "rust-text-b64", "rust-math-b64",
)
METADATA = ("original_page_id", "document_cluster", "cohort", "language", "script",
            "domain", "appearance", "reference_quality", "task")
CSV_FIELDS = ["variant", "cpus", "case_id", *METADATA, "repetition", "sequence",
              "http_status", "error", "successful", "elapsed_us"]
HASH = re.compile(r"[0-9a-f]{64}\Z")
SOURCE = re.compile(r"[0-9a-f]{40}\Z")
STATUSES = {"planned", "not_assigned", "interrupted", "failed", "completed",
            "missing", "rejected", "download_failed"}


class AdmissionError(ValueError):
    """The evidence cannot support a report measurement."""


def require(condition: bool, reason: str) -> None:
    if not condition:
        raise AdmissionError(reason)


def unique_object(pairs: list) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def finite(value):
    if isinstance(value, dict):
        for item in value.values():
            finite(item)
    elif isinstance(value, list):
        for item in value:
            finite(item)
    elif isinstance(value, float):
        require(math.isfinite(value), "non-finite numeric value")
    return value


def decode(data: bytes):
    try:
        return finite(json.loads(data, object_pairs_hook=unique_object))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise AdmissionError(f"invalid JSON: {error}") from error


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def integer(value, name: str) -> int:
    require(type(value) is int and value >= 0, f"invalid integer: {name}")
    return value


class Evidence:
    def __init__(self, root: Path):
        require(root.is_dir() and not root.is_symlink(), "evidence root must be a directory")
        self.root = root.resolve()
        self.hashes = {}
        inventory = self.read("SHA256SUMS", 16 * 1024 * 1024).decode("ascii")
        for line in inventory.splitlines():
            require(len(line) > 66 and line[64:66] == "  ", "invalid inventory row")
            sha, path = line[:64], line[66:]
            require(HASH.fullmatch(sha) is not None and path not in self.hashes,
                    "invalid or duplicate inventory hash/path")
            require(path not in {"SHA256SUMS", "SHA256SUMS.partial"}, "self-indexed inventory")
            require(digest(self.read(path)) == sha, f"inventory hash mismatch: {path}")
            self.hashes[path] = sha
        files = set()
        for path in self.root.rglob("*"):
            require(not path.is_symlink(), "symlinks are not admitted")
            require(path.is_dir() or path.is_file(), "special files are not admitted")
            if path.is_file():
                files.add(path.relative_to(self.root).as_posix())
        require(files == set(self.hashes) | {"SHA256SUMS"}, "unindexed or missing files")

    def path(self, relative: str) -> Path:
        require(isinstance(relative, str) and bool(relative), "missing relative path")
        parts = PurePosixPath(relative).parts
        require(not relative.startswith("/") and "\\" not in relative and
                all(part not in {".", "..", ""} for part in relative.split("/")), "unsafe relative path")
        target = self.root
        for part in parts:
            target = target / part
            require(not target.is_symlink(), "symlink in evidence path")
        require(target.resolve().is_relative_to(self.root), "path escapes evidence")
        require(target.is_file(), f"missing regular file: {relative}")
        return target

    def read(self, relative: str, maximum: int = 128 * 1024 * 1024) -> bytes:
        target = self.path(relative)
        require(target.stat().st_size <= maximum, "evidence file exceeds bound")
        return target.read_bytes()

    def bound(self, relative: str, sha: str, size=None) -> bytes:
        require(isinstance(sha, str) and HASH.fullmatch(sha) is not None, "missing/invalid digest")
        require(self.hashes.get(relative) == sha, f"unindexed digest: {relative}")
        data = self.read(relative)
        require(digest(data) == sha, f"file changed after inventory verification: {relative}")
        if size is not None:
            require(len(data) == integer(size, "byte count"), "byte-count mismatch")
        return data

    def json(self, relative: str, sha=None):
        require(relative in self.hashes, "JSON is not indexed")
        return decode(self.bound(relative, sha or self.hashes[relative]))


def frozen_studies(repository: Path = ROOT) -> dict:
    result = {}
    for cohort, base in [("main", "evaluation"), ("laboratory", "evaluation/laboratory-supplement")]:
        manifest_bytes = (repository / base / "corpus-frozen.json").read_bytes()
        protocol_bytes = (repository / base / "protocol.json").read_bytes()
        manifest, protocol = decode(manifest_bytes), decode(protocol_bytes)
        require(digest(manifest_bytes) == protocol["corpus_sha256"], "local frozen corpus hash mismatch")
        require([v["id"] for v in protocol["primary_variants"]] == list(VARIANTS), "unexpected frozen variants")
        result[cohort] = {"manifest": manifest, "protocol": protocol,
                          "manifest_sha256": digest(manifest_bytes),
                          "protocol_sha256": digest(protocol_bytes),
                          "manifest_bytes": len(manifest_bytes)}
    return result


def missing_cells() -> list[dict]:
    return [{"cohort": cohort, "variant": variant, "cpus": cpus,
             "status": "unsubmitted", "reason": "No admitted execution artifact was supplied."}
            for cohort, cpus, variant in itertools.product(("main", "laboratory"), (1, 2), VARIANTS)]


def load(root: Path, source_sha: str, repository: Path = ROOT) -> dict:
    """Validate a combined artifact against reviewer-selected source and freezes.

    Failed/incomplete runs remain in statuses but cannot contribute measurements.
    Caller must retain the complete returned status table beside any plot.
    """
    try:
        require(SOURCE.fullmatch(source_sha) is not None, "expected source must be a full commit SHA")
        evidence = Evidence(root)
        index = evidence.json("study-index.json")
        require(index["schema_version"] == 1 and index["source_sha"] == source_sha, "source/schema mismatch")
        require(index["assigned_cpu_budget"] is None, "report requires combined artifact")
        require(index["status"] in {"partial", "measured"}, "invalid combined status")
        studies = frozen_studies(repository)
        require(len(index["studies"]) == 2 and {s["cohort"] for s in index["studies"]} == set(studies),
                "both frozen studies required")
        for study in index["studies"]:
            expected = studies[study["cohort"]]
            protocol = expected["protocol"]
            for key in ("manifest_sha256", "protocol_sha256"):
                require(study[key] == expected[key], f"frozen {key} mismatch")
            require(study["selection_seed"] == protocol["selection_seed"] and
                    study["bootstrap_seed"] == protocol["metrics"]["bootstrap_seed"] and
                    study["bootstrap_resamples"] == protocol["metrics"]["bootstrap_resamples"], "study seed/resampling mismatch")
            for key in ("manifest", "protocol"):
                evidence.bound(study[key], study[key + "_sha256"])
        require(index["manifest_sha256"] == studies["main"]["manifest_sha256"] and
                index["protocol_sha256"] == studies["main"]["protocol_sha256"], "main identity mismatch")
        expected_keys = {(r["cohort"], r["variant"], r["cpus"]) for r in missing_cells()}
        runs = index["runs"]
        require(len(runs) == 32 and {(r["cohort"], r["variant"], r["cpus"]) for r in runs} == expected_keys,
                "expected exactly 32 unique cells")
        require(all(type(r["cpus"]) is int and type(r["sequence"]) is int for r in runs), "non-integer cell identity")
        require({r["sequence"] for r in runs} == set(range(1, 33)), "invalid/duplicate sequence")
        provenance = index["provenance"]
        require(provenance["repository"] == "somehowchris/tika-indra-ocr-client", "repository mismatch")
        for key in ("release_run_id", "release_run_attempt"):
            require(re.fullmatch(r"[1-9][0-9]*", provenance[key]) is not None, "release identity missing")
        shards = provenance["shards"]
        require(len(shards) == 2 and {s["cpus"] for s in shards} == {1, 2}, "missing/duplicate budget shard")
        identities = []
        for shard in shards:
            if shard["status"] != "admitted":
                require(not any(r["status"] == "completed" for r in runs if r["cpus"] == shard["cpus"]),
                        "completed run in unadmitted shard")
                continue
            shard_index = evidence.json(shard["index"], shard["index_sha256"])
            require(shard_index["source_sha"] == source_sha and shard_index["assigned_cpu_budget"] == shard["cpus"], "shard identity mismatch")
            require(shard_index["provenance"] == shard["provenance"], "shard provenance mismatch")
            prefix = str(PurePosixPath(shard["index"]).parent) + "/"
            p = shard["provenance"]
            require(p["repository"] == provenance["repository"] and
                    all(p[k] == provenance[k] for k in ("release_run_id", "release_run_attempt")), "shard release mismatch")
            for name in ("candidates", "bundle", "evaluator", "evaluate_script", "schedule"):
                evidence.bound(prefix + p[name], p[name + "_sha256"])
            identities.append(tuple(p[k + "_sha256"] for k in ("candidates", "bundle", "evaluator", "evaluate_script")))
            candidates = evidence.json(prefix + p["candidates"], p["candidates_sha256"])
            require(candidates["source_sha"] == source_sha and candidates["repository"] == provenance["repository"] and
                    all(candidates[k] == provenance[k] for k in ("release_run_id", "release_run_attempt")), "candidate release mismatch")
            require(len(candidates["images"]) == 6 and {i["role"] for i in candidates["images"]} ==
                    {"native", "gateway", "bridge", "tika", "tesseract", "backend"}, "candidate roles mismatch")
            for image in candidates["images"]:
                require(re.fullmatch(r"sha256:[0-9a-f]{64}", image["digest"]) is not None and
                        image["reference"] == "ghcr.io/somehowchris/tika-indra-" + image["role"] + "@" + image["digest"], "candidate digest/reference mismatch")
            if any(r["status"] == "completed" for r in runs if r["cpus"] == shard["cpus"]):
                images = evidence.json(prefix + p["images"], p["images_sha256"])
                require(len(images) == 6 and {i["role"] for i in images} == {i["role"] for i in candidates["images"]}, "executed image inventory mismatch")
                for image in images:
                    candidate = next(c for c in candidates["images"] if c["role"] == image["role"])
                    require(image["reference"] == candidate["reference"], "executed candidate reference mismatch")
                    identity = evidence.json(prefix + image["resolution"] + "/identity.json", image["resolution_sha256"])
                    require(identity["passed"] is True and identity["root_digest"] == candidate["digest"] and
                            identity["image_id"] == image["image_id"] and identity["os"] == "linux" and identity["architecture"] == "amd64", "OCI execution identity mismatch")
            for run in (r for r in runs if r["cpus"] == shard["cpus"]):
                original = next((r for r in shard_index["runs"] if r["sequence"] == run["sequence"]), None)
                require(original is not None, "missing original shard run")
                rewritten = dict(original)
                for key in ("scores", "performance", "attempts", "raw_run", "driver_result", "parent_before", "parent_after", "parent_counters_after"):
                    if rewritten.get(key) is not None:
                        rewritten[key] = prefix + rewritten[key]
                require(rewritten == run, "combined run differs from shard")
        require(len(set(identities)) <= 1, "CPU budgets used different candidate/evaluator bytes")
        measurements = []
        parents = set()
        for run in runs:
            require(run["status"] in STATUSES, "unknown run status")
            for key, hash_key in [("scores", "sha256"), ("performance", "performance_sha256"),
                                  ("attempts", "attempts_sha256"), ("driver_result", "driver_result_sha256")]:
                require((run[key] is None) == (run[hash_key] is None), "unpaired artifact identity")
                if run[key] is not None:
                    evidence.bound(run[key], run[hash_key])
            if run["status"] != "completed":
                continue
            require(all(run[k] == 0 and type(run[k]) is int for k in ("driver_exit", "score_exit", "image_binding_exit")), "completed run has failed gate")
            measured = validate_run(evidence, run, studies[run["cohort"]])
            parent = (run["cpus"], measured["parent"]["device"], measured["parent"]["inode"])
            require(parent not in parents, "cgroup parent reused between cells")
            parents.add(parent)
            measurements.append(measured)
        comparisons = validate_comparisons(evidence, index, studies)
        require(index["status"] != "measured" or (len(measurements) == 32 and len(comparisons) == 112), "measured study incomplete")
        return {"index_sha256": evidence.hashes["study-index.json"], "source_sha": source_sha,
                "statuses": [{k: r.get(k) for k in ("cohort", "variant", "cpus", "status", "reason")} for r in runs],
                "measurements": measurements, "comparisons": comparisons}
    except (KeyError, TypeError, StopIteration, UnicodeError, OSError, csv.Error, ValueError) as error:
        raise AdmissionError(f"malformed/incomplete evidence: {error}") from error


def validate_run(e: Evidence, run: dict, study: dict) -> dict:
    score = e.json(run["scores"], run["sha256"])
    performance = e.json(run["performance"], run["performance_sha256"])
    driver = e.json(run["driver_result"], run["driver_result_sha256"])
    require(driver["complete"] is True and driver["label"] == run["variant"] and
            driver["execution_error"] is None and driver["integrity_error"] is None and not driver["cleanup_errors"], "driver is incomplete")
    cases = {c["id"]: c for c in study["manifest"]["cases"]}
    require(len(score["cases"]) == len(cases) and {c["case_id"] for c in score["cases"]} == set(cases), "score case selection differs")
    for report in (score, performance):
        require(report["schema_version"] == 1 and report["variant"] == run["variant"] and report["cpus"] == run["cpus"] and
                report["manifest_sha256"] == study["manifest_sha256"] and report["manifest_bytes"] == study["manifest_bytes"] and
                report["corpus_id"] == study["manifest"]["corpus_id"] and report["journal_complete"] is True and
                report["truncated_final_record_bytes"] == 0 and report["expected_repetitions"] == 3, "score/performance identity mismatch")
    require(score["selected_accuracy_repetition"] == 1 and score["selection_seed"] == study["manifest"]["selection_seed"], "first-attempt policy mismatch")
    require(score["journal_sha256"] == performance["journal_sha256"] and score["journal_bytes"] == performance["journal_bytes"], "journal mismatch")
    journal = e.bound(run["raw_run"] + "/requests.jsonl", score["journal_sha256"], score["journal_bytes"])
    require(journal.endswith(b"\n"), "truncated journal")
    records = [decode(line) for line in journal.splitlines()]
    require(records and records[0].get("kind") == "environment" and records[-1].get("kind") == "resource_end", "journal boundaries missing")
    environment = records[0]
    require(sum(r.get("kind") == "environment" for r in records) == 1 and sum(r.get("kind") == "resource_end" for r in records) == 1, "duplicate journal boundary")
    require(environment["variant"] == run["variant"] and environment["corpus_id"] == study["manifest"]["corpus_id"] and
            environment["manifest_sha256"] == study["manifest_sha256"] and environment["manifest_bytes"] == study["manifest_bytes"] and
            environment["selection_seed"] == study["manifest"]["selection_seed"] and environment["repetitions"] == 3, "journal environment mismatch")
    for boundary in (environment, records[-1]):
        budget = boundary["budget"]
        require(budget["cpus"] == run["cpus"] and budget["quota_us"] == budget["period_us"] * run["cpus"] and
                budget["period_us"] > 0 and budget["memory_limit_bytes"] == 3 * 1024**3, "journal resource budget mismatch")
    require(sum(r.get("kind") == "warmup" for r in records) == score["warmup_attempts"] == performance["warmup_attempts_excluded"] == 1,
            "warmup count mismatch")
    attempts = [r for r in records if r.get("kind") == "attempt"]
    rows = list(csv.DictReader(io.StringIO(e.read(run["attempts"]).decode())))
    require(rows and list(rows[0]) == CSV_FIELDS and len(rows) == len(cases) * 3 == len(attempts), "attempt CSV shape/count mismatch")
    by_identity = {}
    for attempt in attempts:
        key = (attempt["case_id"], attempt["repetition"])
        require(key not in by_identity and key[0] in cases and type(key[1]) is int and 1 <= key[1] <= 3, "duplicate/unknown journal attempt")
        require(attempt["variant"] == run["variant"] and attempt["cpus"] == run["cpus"], "attempt run identity mismatch")
        require(all(attempt[k] == cases[key[0]][k] for k in METADATA), "attempt metadata mismatch")
        e.bound(run["raw_run"] + "/" + attempt["output_path"], attempt["output_sha256"], attempt["output_bytes"])
        integer(attempt["elapsed_us"], "elapsed_us")
        by_identity[key] = attempt
    seen = set()
    for row in rows:
        key = (row["case_id"], int(row["repetition"]))
        require(key not in seen and key in by_identity, "duplicate/unknown CSV attempt")
        seen.add(key)
        a = by_identity[key]
        for name in CSV_FIELDS:
            expected = a.get(name)
            if name == "successful":
                expected = a["status"] is not None and 200 <= a["status"] < 300 and a["error"] is None
            elif name == "http_status":
                expected = a["status"]
            require(row[name] == ("" if expected is None else str(expected).lower() if isinstance(expected, bool) else str(expected)), f"CSV/journal mismatch: {name}")
    for case in score["cases"]:
        frozen = cases[case["case_id"]]
        require(all(case[k] == frozen[k] for k in METADATA) and case["word_tokenizer"] == frozen["word_tokenizer"] and
                case["input_sha256"] == frozen["input"]["sha256"] and case["reference_sha256"] == frozen["reference"]["sha256"], "score/frozen case mismatch")
        first = by_identity[(case["case_id"], 1)]
        require(case["first_output_sha256"] == first["output_sha256"] and case["first_output_bytes"] == first["output_bytes"] and
                case["first_elapsed_us"] == first["elapsed_us"] and case["first_http_status"] == first["status"], "score first-attempt mismatch")
        require((case["metrics"] is not None) == (case["outcome"] == "success"), "score outcome/metrics mismatch")
        require(case["outcome"] in {"success", "service_failure", "metric_limit"}, "invalid complete-run outcome")
        first_success = first["status"] is not None and 200 <= first["status"] < 300 and first["error"] is None
        require((case["outcome"] == "service_failure") == (not first_success), "first failure repaired into success")
        if case["metrics"] is not None:
            require(set(case["metrics"]) == {"cer", "cer_whitespace_collapsed", "wer"}, "invalid metric fields")
            for metric, distance in case["metrics"].items():
                if metric == "wer" and frozen["word_tokenizer"] != "whitespace":
                    require(distance is None, "unsupported WER tokenizer")
                    continue
                require(isinstance(distance, dict) and set(distance) == {"reference_units", "hypothesis_units", "errors"}, "missing metric distance")
                reference, hypothesis, errors = (integer(distance[k], k) for k in ("reference_units", "hypothesis_units", "errors"))
                require(abs(reference - hypothesis) <= errors <= max(reference, hypothesis), "impossible edit-distance counts")
    validate_summaries(score)
    successful = sum(r["successful"] == "true" for r in rows)
    expected = len(rows)
    require(performance["selected_cases"] == len(cases) and performance["expected_attempts"] == expected and
            performance["observed_attempts"] == expected and performance["missing_attempts"] == 0 and
            performance["successful_attempts"] == successful and performance["failed_attempts"] == expected - successful, "performance count mismatch")
    endings = [r for r in records if r.get("kind") == "repetition_end" and r.get("repetition") != 0]
    require(len(endings) == 3 and {r["repetition"] for r in endings} == {1, 2, 3}, "incomplete repetition boundaries")
    for end in endings:
        selected = [a for a in attempts if a["repetition"] == end["repetition"]]
        require(end["attempted_pages"] == len(cases) and {a["sequence"] for a in selected} == set(range(len(cases))) and
                end["wall_us"] >= sum(a["elapsed_us"] for a in selected), "repetition count/sequence/wall mismatch")
    require(len(performance["completed_repetitions"]) == 3, "performance repetition count mismatch")
    for repetition in performance["completed_repetitions"]:
        end = next(e for e in endings if e["repetition"] == repetition["repetition"])
        selected = [a for a in attempts if a["repetition"] == end["repetition"]]
        successes = sum(a["status"] is not None and 200 <= a["status"] < 300 and a["error"] is None for a in selected)
        require(repetition["wall_us"] == end["wall_us"] and repetition["attempts"] == len(selected) and
                repetition["successful_attempts"] == successes and repetition["failed_attempts"] == len(selected) - successes and
                repetition["sum_attempt_elapsed_us"] == sum(a["elapsed_us"] for a in selected), "performance repetition mismatch")
    wall = sum(integer(r["wall_us"], "wall_us") for r in endings)
    require(wall > 0 and performance["completed_repetition_wall_us"] == wall and
            math.isclose(performance["successful_case_responses_per_complete_workload_wall_second"], successful * 1_000_000 / wall, rel_tol=1e-12), "throughput denominator mismatch")
    before, after = e.json(run["parent_before"]), e.json(run["parent_after"])
    require(before == after and before["memory_max"] == 3 * 1024**3 and before["memory_swap_max"] == 0, "cgroup budget changed")
    quota, period = map(int, before["cpu_max"].split())
    require(quota == run["cpus"] * period and period > 0, "CPU quota mismatch")
    return {"cohort": run["cohort"], "variant": run["variant"], "cpus": run["cpus"],
            "score": score, "performance": performance, "parent": before,
            "timings": [{k: a[k] for k in ("case_id", "cohort", "repetition", "elapsed_us", "status", "error")} for a in attempts]}


def validate_summaries(score: dict) -> None:
    axes = ("overall", "language", "script", "domain", "appearance", "reference_quality")
    cases = score["cases"]
    expected = {(c["cohort"], axis, "all" if axis == "overall" else c[axis]) for c in cases for axis in axes}
    summaries = score["summaries"]
    require(len(summaries) == len(expected) and {(s["cohort"], s["axis"], s["level"]) for s in summaries} == expected, "summary strata mismatch")
    for summary in summaries:
        selected = [c for c in cases if c["cohort"] == summary["cohort"] and (summary["axis"] == "overall" or c[summary["axis"]] == summary["level"])]
        scored = [c for c in selected if c["outcome"] == "success"]
        require(summary["selected_cases"] == len(selected) and summary["scored_cases"] == len(scored) and
                summary["service_failures"] == sum(c["outcome"] == "service_failure" for c in selected) and
                summary["metric_limits"] == sum(c["outcome"] == "metric_limit" for c in selected) and
                summary["missing_first_attempts"] == 0 and summary["document_clusters"] == len({c["document_cluster"] for c in selected}) and
                summary["scored_document_clusters"] == len({c["document_cluster"] for c in scored}), "summary coverage mismatch")
        for metric in ("cer", "cer_whitespace_collapsed", "wer"):
            distances = [c["metrics"][metric] for c in scored if c["metrics"][metric] is not None]
            aggregate = summary[metric]
            ref = sum(d["reference_units"] for d in distances)
            errors = sum(d["errors"] for d in distances)
            rates = [d["errors"] / d["reference_units"] for d in distances if d["reference_units"]]
            require(aggregate["cases"] == len(distances) and aggregate["reference_units"] == ref and
                    aggregate["errors"] == errors and aggregate["hypothesis_units"] == sum(d["hypothesis_units"] for d in distances) and
                    aggregate["empty_reference_cases"] == len(distances) - len(rates), "summary metric counts mismatch")
            for name, value in [("micro_rate", errors / ref if ref else None), ("macro_case_rate", sum(rates) / len(rates) if rates else None)]:
                require((aggregate[name] is None) == (value is None) and (value is None or math.isclose(aggregate[name], value, abs_tol=1e-12, rel_tol=1e-12)), "summary rate mismatch")


def validate_comparisons(e: Evidence, index: dict, studies: dict) -> list:
    expected = {(c, n, l, r) for c, n in itertools.product(studies, (1, 2)) for l, r in itertools.combinations(VARIANTS, 2)}
    rows = index["comparisons"]
    require(len(rows) == 112 and {(r["cohort"], r["cpus"], r["left"], r["right"]) for r in rows} == expected, "comparison selection mismatch")
    accepted = []
    for row in rows:
        require(row["status"] in STATUSES | {"skipped"}, "unknown comparison status")
        require((row["path"] is None) == (row["sha256"] is None), "unpaired comparison identity")
        if row["path"] is not None:
            e.bound(row["path"], row["sha256"])
        if row["status"] != "completed":
            continue
        require(row["exit_code"] == 0, "comparison command failed")
        arms = [next(r for r in index["runs"] if r["cohort"] == row["cohort"] and r["cpus"] == row["cpus"] and r["variant"] == v) for v in (row["left"], row["right"])]
        require(all(r["status"] == "completed" for r in arms), "comparison pairs incomplete runs")
        report = e.json(row["path"], row["sha256"])
        protocol = studies[row["cohort"]]["protocol"]
        require(report["manifest_sha256"] == studies[row["cohort"]]["manifest_sha256"] and
                report["bootstrap_seed"] == protocol["metrics"]["bootstrap_seed"] and
                report["requested_bootstrap_samples"] == protocol["metrics"]["bootstrap_resamples"], "comparison protocol mismatch")
        for side, arm in zip(("left", "right"), arms, strict=True):
            require(report[side + "_variant"] == arm["variant"] and report[side + "_cpus"] == arm["cpus"] and
                    report[side + "_scores_sha256"] == arm["sha256"] and
                    report[side + "_scores_bytes"] == len(e.read(arm["scores"])), "comparison arm identity mismatch")
        if row["cohort"] == "laboratory":
            require(all(s[m]["ci_95_low"] is None and s[m]["ci_95_high"] is None and s[m]["bootstrap_samples"] == 0
                        for s in report["strata"] for m in ("cer", "cer_whitespace_collapsed", "wer")), "laboratory intervals forbidden")
        accepted.append({"cohort": row["cohort"], "cpus": row["cpus"], "report": report})
    return accepted
