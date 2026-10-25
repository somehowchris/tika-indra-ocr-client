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
    "tesseract-best-psm11", "python-math-b64", "rust-math-b64",
)
METADATA = ("original_page_id", "document_cluster", "cohort", "language", "script",
            "domain", "appearance", "reference_quality", "task")
# The evaluator's journal attempt names the case's metadata except its original
# page id, which only the frozen manifest and the scorer's attempt CSV carry.
JOURNAL_METADATA = METADATA[1:]
JOURNAL_ATTEMPT_FIELDS = frozenset((
    "kind", "variant", "cpus", "repetition", "sequence", "case_id", "writer_cluster", "tesseract_language",
    "tesseract_psm", "elapsed_us", "status", "error", "output_path", "output_bytes", "output_sha256", *JOURNAL_METADATA))
CSV_FIELDS = ["variant", "cpus", "case_id", *METADATA, "repetition", "sequence",
              "http_status", "error", "successful", "elapsed_us"]
HASH = re.compile(r"[0-9a-f]{64}\Z")
SOURCE = re.compile(r"[0-9a-f]{40}\Z")
STATUSES = {"planned", "not_assigned", "interrupted", "failed", "completed",
            "missing", "rejected", "download_failed"}
# The report states its results under this amendment; the protocols name no
# model, so only each shard's own model lock tells the model sets apart.
MODEL_AMENDMENT = "evaluation/audits/indra-1-1-20261020/amendment.json"
MODEL_LOCK = "provenance/source/backend/models.lock.json"


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


def unsubmitted() -> dict:
    """The report state without an artifact: every planned cell and pair, nothing measured."""
    return {"statuses": missing_cells(), "measurements": [], "comparisons": [],
            "comparison_statuses": missing_comparisons(), "hosts": []}


COMPARISON_FIELDS = ("cohort", "cpus", "left", "right", "status", "reason", "exit_code", "path", "sha256")


def missing_comparisons() -> list[dict]:
    return [{"cohort": cohort, "cpus": cpus, "left": left, "right": right,
             "status": "unsubmitted", "reason": "No admitted execution artifact was supplied.",
             "exit_code": None, "path": None, "sha256": None}
            for cohort, cpus in itertools.product(("main", "laboratory"), (1, 2))
            for left, right in itertools.combinations(VARIANTS, 2)]


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
        model = decode((repository / MODEL_AMENDMENT).read_bytes())["model_binding"]["after"]
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
        require(len(runs) == 24 and {(r["cohort"], r["variant"], r["cpus"]) for r in runs} == expected_keys,
                "expected exactly 24 unique cells")
        require(all(type(r["cpus"]) is int and type(r["sequence"]) is int for r in runs), "non-integer cell identity")
        require({r["sequence"] for r in runs} == set(range(1, 25)), "invalid/duplicate sequence")
        provenance = index["provenance"]
        require(provenance["repository"] == "somehowchris/tika-indra-ocr-client", "repository mismatch")
        for key in ("release_run_id", "release_run_attempt"):
            require(re.fullmatch(r"[1-9][0-9]*", provenance[key]) is not None, "release identity missing")
        shards = provenance["shards"]
        require(len(shards) == 2 and {s["cpus"] for s in shards} == {1, 2}, "missing/duplicate budget shard")
        require(re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", provenance["aggregated_utc"]) is not None, "aggregation time missing")
        identities, hosts = [], []
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
            lock = evidence.json(prefix + MODEL_LOCK)
            require(lock["revision"] == model["revision"] and lock["package_version"] == model["package"]["version"],
                    "model lock is not the Indra 1.1 amendment revision")
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
            rust = [r for r in runs if (r["cpus"], r["variant"], r["status"]) == (shard["cpus"], VARIANTS[-1], "completed")]
            hosts.append(host(evidence, prefix, shard["cpus"], rust))
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
        require(index["status"] != "measured" or (len(measurements) == 24 and len(comparisons) == 60), "measured study incomplete")
        return {"index_sha256": evidence.hashes["study-index.json"], "source_sha": source_sha,
                "release_run_id": provenance["release_run_id"], "aggregated_utc": provenance["aggregated_utc"],
                "hosts": sorted(hosts, key=lambda h: h["cpus"]),
                "statuses": [{k: r.get(k) for k in ("cohort", "variant", "cpus", "status", "reason")} for r in runs],
                "measurements": measurements, "comparisons": comparisons,
                "comparison_statuses": [{k: row.get(k) for k in COMPARISON_FIELDS} for row in index["comparisons"]]}
    except (KeyError, TypeError, StopIteration, UnicodeError, OSError, csv.Error, ValueError) as error:
        raise AdmissionError(f"malformed/incomplete evidence: {error}") from error


def known(value) -> str:
    return "unknown" if value is None else str(value)


HOST_FIELDS = ("cpus", "host", "kernel", "cpu_model", "avx512f", "selected_cpus", "physical_cores",
               "core_ids", "thread_siblings", "process_allowed_cpus", "rust_isa_avx512f", "rust_opencv_features")
OCR_LOG = re.compile(r"\d{4}-ocr-logs\.stderr\Z")


def dispatch(e: Evidence, runs: list) -> tuple:
    """The Rust arm's startup cpu_dispatch record: CPUID AVX-512F as its OCR
    process saw it and OpenCV's feature line, where * marks an extension built
    for runtime dispatch and ? one the CPU lacks. ONNX Runtime logs no choice.
    Unknown (None) when no completed Rust run of the budget logged it."""
    records = set()
    for run in runs:
        commands = str(PurePosixPath(run["raw_run"]).parent) + "/commands/"
        logs = [path for path in e.hashes if path.startswith(commands) and OCR_LOG.fullmatch(path[len(commands):])]
        require(len(logs) <= 1, "several OCR log captures in one run")
        for path in logs:
            for line in e.bound(path, e.hashes[path]).decode().splitlines():
                if '"cpu_dispatch"' in line:
                    fields = decode(line[line.index("{"):].encode())["fields"]
                    if fields.get("event") == "cpu_dispatch":
                        require(type(fields["isa_avx512f"]) is bool and type(fields["opencv_features"]) is str, "invalid dispatch record")
                        records.add((fields["isa_avx512f"], fields["opencv_features"]))
    require(len(records) <= 1, "Rust arm dispatch differs within a budget")
    return records.pop() if records else (None, None)


def host(e: Evidence, prefix: str, cpus: int, rust: list) -> dict:
    """The logical CPUs of one budget's shard, from that shard's bound host records."""
    text = lambda name: e.bound(prefix + name, e.hashes.get(prefix + name)).decode()
    affinity = e.json(prefix + "provenance/affinity.json")
    processors = {}
    for block in text("provenance/cpuinfo.txt").split("\n\n"):
        fields = {key.strip(): value.strip() for key, _, value in (line.partition(":") for line in block.splitlines())}
        if "processor" in fields:
            processors[int(fields["processor"])] = fields
    selected = affinity["selected_cpu_one" if cpus == 1 else "selected_cpu_two"]
    numbers = [int(n) for n in selected.split(",")]
    topology = {t["cpu"]: t for t in affinity["selected_cpu_topology"]}
    require(len(numbers) == cpus and all(n in processors and n in topology for n in numbers), "budget CPUs missing from host records")
    models = {processors[n]["model name"] for n in numbers}
    require(len(models) == 1, "budget CPUs report different models")
    # sysfs prints one sibling list per physical core; null when unreadable.
    siblings = [topology[n]["thread_siblings_list"] for n in numbers]
    uname = text("control/host-uname.stdout").split()
    require(len(uname) >= 3, "host uname unreadable")
    isa_avx512f, opencv_features = dispatch(e, rust)
    return {"cpus": cpus, "host": uname[1], "kernel": uname[2], "cpu_model": models.pop(),
            "avx512f": all("avx512f" in processors[n]["flags"].split() for n in numbers), "selected_cpus": selected,
            "physical_cores": None if None in siblings else len(set(siblings)),
            "core_ids": ",".join(known(topology[n]["core_id"]) for n in numbers), "thread_siblings": ";".join(map(known, siblings)),
            "process_allowed_cpus": affinity["process_allowed_cpus"], "rust_isa_avx512f": isa_avx512f, "rust_opencv_features": opencv_features}


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
        require(set(attempt) == JOURNAL_ATTEMPT_FIELDS, "journal attempt fields differ from the evaluator's record")
        require(all(attempt[k] == cases[key[0]][k] for k in JOURNAL_METADATA) and
                attempt["writer_cluster"] == cases[key[0]].get("writer_cluster"), "attempt metadata mismatch")
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
            if name in METADATA:
                expected = cases[key[0]][name]
            elif name == "successful":
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
    resources = validate_resources(e, run, records, before)
    return {"cohort": run["cohort"], "variant": run["variant"], "cpus": run["cpus"],
            "score": score, "performance": performance, "parent": before,
            "timings": [{k: a[k] for k in ("case_id", "cohort", "repetition", "elapsed_us", "status", "error")} for a in attempts],
            "resources": resources}


RESOURCE_FIELDS = (
    "startup_ready_ms", "backend_poll_ms", "gateway_poll_ms",
    "warmup_workload_cpu_usage_usec", "warmup_workload_cpu_user_usec", "warmup_workload_cpu_system_usec",
    "cpu_since_admission_usage_usec", "cpu_counters_warmup_workload_delta", "cpu_counters_since_admission_delta",
    "admission_memory_current_bytes", "before_warmup_memory_current_bytes", "after_workload_memory_current_bytes", "after_driver_memory_current_bytes",
    "admission_memory_peak_bytes", "before_warmup_memory_peak_bytes", "after_workload_memory_peak_bytes", "parent_memory_peak_observed_bytes",
    "memory_events_admission", "memory_events_before_warmup", "memory_events_after_workload", "memory_events_after_driver",
    "memory_events_warmup_workload_delta", "memory_events_since_admission_delta",
)


def resource_integer(value, name, bits=64):
    require(integer(value, name) < 2**bits, "resource integer exceeds unsigned range: " + name)
    return value


def counters(text, name, required):
    require(isinstance(text, str) and 0 < len(text.encode()) <= 4096 and text.endswith("\n"), "invalid counter record: " + name)
    result = {}
    for line in text[:-1].split("\n"):
        match = re.fullmatch(r"([a-z][a-z0-9_.]*) (0|[1-9][0-9]*)", line)
        require(match is not None, "invalid counter line: " + name)
        key, value = match.groups()
        require(key not in result, "duplicate counter: " + name)
        result[key] = resource_integer(int(value), name + "." + key)
    require(set(required) <= set(result), "missing counter: " + name)
    return result


def counter_delta(before, after, name):
    require(before.keys() == after.keys() and all(after[k] >= before[k] for k in before), "decreasing/changed counter set: " + name)
    return {key: after[key] - value for key, value in before.items()}


def validate_resources(e, run, records, parent):
    # Bind actual sibling records from the producing Rust driver; no new samples.
    before_path = PurePosixPath(run["parent_before"])
    require(before_path.name == "parent-before.json", "unexpected driver resource path")
    directory = before_path.parent
    sibling = lambda name: str(directory / name)
    require(run["parent_after"] == sibling("parent-after.json") and run["parent_counters_after"] == sibling("parent-counters-after.json") and
            run["raw_run"] == sibling("run"), "resource records are from different driver directories")
    readiness = e.json(sibling("readiness.json"))
    initial, final = e.json(sibling("parent-freshness.json")), e.json(run["parent_counters_after"])
    boundaries = [records[0]["budget"], records[-1]["budget"]]
    require(re.fullmatch(r"[1-9][0-9]* [1-9][0-9]*\n", parent["cpu_max"]) is not None, "noncanonical parent CPU budget")
    quota, period = (resource_integer(int(v), "parent cpu.max") for v in parent["cpu_max"].split())
    require(quota == run["cpus"] * period and type(parent["memory_max"]) is int and parent["memory_max"] == 3*1024**3 and
            type(parent["memory_swap_max"]) is int and parent["memory_swap_max"] == 0, "parent resource budget differs")
    for boundary in boundaries:
        for key in ("cpus", "quota_us", "period_us", "memory_limit_bytes"):
            resource_integer(boundary[key], key)
        require(boundary["cpus"] == run["cpus"] and boundary["quota_us"] == quota and boundary["period_us"] == period and
                boundary["memory_limit_bytes"] == parent["memory_max"] and boundary["parent_path"] == parent["path"], "journal/parent resource identity differs")
    snapshots = [initial, *boundaries, final]
    cpu = [counters(row["cpu_stat"], "cpu.stat", ("usage_usec", "user_usec", "system_usec")) for row in snapshots]
    events = [counters(row["memory_events"], "memory.events", ("low", "high", "max", "oom", "oom_kill")) for row in snapshots]
    for left, right in zip(cpu, cpu[1:]):
        counter_delta(left, right, "cpu.stat")
    for left, right in zip(events, events[1:]):
        counter_delta(left, right, "memory.events")
    current, peak = [], []
    for row in snapshots:
        require(all(key in row for key in ("memory_current_bytes", "memory_peak_bytes")), "missing charged-memory snapshot")
        current.append(resource_integer(row["memory_current_bytes"], "memory.current"))
        value = row["memory_peak_bytes"]
        require(value is None or resource_integer(value, "memory.peak") >= current[-1], "peak below current charged memory")
        peak.append(value)
    require(peak[0] is not None and peak[-1] is not None, "driver lifetime peak is missing")
    observed = [p for p in peak if p is not None]
    require(observed == sorted(observed), "parent lifetime peak decreased")
    for row, parsed in ((initial, cpu[0]), (final, cpu[-1])):
        require(resource_integer(row["cpu_usage_usec"], "driver usage_usec") == parsed["usage_usec"], "driver/raw CPU counter mismatch")
        resource_integer(row["descendants"], "descendants")
        resource_integer(row["dying_descendants"], "dying descendants")
    ready = resource_integer(readiness["aggregate_start_to_ready_ms"], "startup readiness", 128)
    gateway = resource_integer(readiness["gateway_poll_ms"], "gateway readiness", 128)
    backend = readiness["backend_poll_ms"]
    require(backend is None or type(backend) is int, "invalid backend readiness")
    require(ready >= gateway + (0 if backend is None else resource_integer(backend, "backend readiness", 128)), "readiness windows disagree")
    work_cpu = counter_delta(cpu[1], cpu[2], "warmup/workload CPU")
    lifetime_cpu = counter_delta(cpu[0], cpu[3], "captured CPU since admission")
    encode = lambda value: json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
    return {"startup_ready_ms": ready, "backend_poll_ms": backend, "gateway_poll_ms": gateway,
            **{"warmup_workload_cpu_" + key: work_cpu[key] for key in ("usage_usec", "user_usec", "system_usec")},
            "cpu_since_admission_usage_usec": lifetime_cpu["usage_usec"],
            "cpu_counters_warmup_workload_delta": encode(work_cpu), "cpu_counters_since_admission_delta": encode(lifetime_cpu),
            **{name + "_memory_current_bytes": value for name, value in zip(("admission", "before_warmup", "after_workload", "after_driver"), current)},
            **{name: value for name, value in zip(("admission_memory_peak_bytes", "before_warmup_memory_peak_bytes", "after_workload_memory_peak_bytes", "parent_memory_peak_observed_bytes"), peak)},
            **{"memory_events_" + name: encode(value) for name, value in zip(("admission", "before_warmup", "after_workload", "after_driver"), events)},
            "memory_events_warmup_workload_delta": encode(counter_delta(events[1], events[2], "warmup/workload memory.events")),
            "memory_events_since_admission_delta": encode(counter_delta(events[0], events[3], "captured memory.events since admission"))}


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
    require(len(rows) == 60 and {(r["cohort"], r["cpus"], r["left"], r["right"]) for r in rows} == expected, "comparison selection mismatch")
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
