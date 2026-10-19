#!/usr/bin/env python3
"""Independent, offline arithmetic replay; never an OCR or trust-admission tool.

The Rust scorer uses a scalar row DP. This checker uses arbitrary-width Myers
bit vectors and Python's independently maintained Unicode 16 NFC/category data.
It does not import the Rust implementation or the report validation modules.
"""
from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import math
import os
from pathlib import Path, PurePosixPath
import platform
import re
import subprocess
import sys
import unicodedata as ucd

UCD = "16.0.0"
RUST_UCD = [17, 0, 0]  # Pinned rustc 1.99.1 standalone probe; White_Space set below.
LIMIT = 32768
METRICS = ("cer", "cer_whitespace_collapsed", "wer")
AXES = ("overall", "language", "script", "domain", "appearance", "reference_quality")
META = ("original_page_id", "document_cluster", "cohort", "language", "script", "domain",
        "appearance", "reference_quality", "task")
VARIANTS = ("tesseract-fast-psm3", "tesseract-best-psm3", "tesseract-best-psm6",
            "tesseract-best-psm11", "python-math-b64", "rust-math-b64")
WHITE_SPACE = frozenset(map(chr, (*range(9, 14), 32, 133, 160, 5760,
                                      *range(8192, 8203), 8232, 8233, 8239, 8287, 12288)))
MASK = (1 << 64) - 1
HASH = re.compile(r"[0-9a-f]{64}\Z")
SOURCE = re.compile(r"[0-9a-f]{40}\Z")


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def uint(value, bits, reason):
    require(type(value) is int and 0 <= value < (1 << bits), reason)
    return value


def finite_float(text):
    value = float(text)
    require(math.isfinite(value), "non-finite JSON float")
    return value


def invalid_constant(value):
    raise ValueError("non-finite JSON constant: " + value)


def decode(data):
    return json.loads(data.decode("utf-8"), object_pairs_hook=unique, parse_constant=invalid_constant, parse_float=finite_float)


class Files:
    """Hash only consumed, bounded text records; do not rehash models or images."""
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.consumed = {}

    def read(self, name, expected=None, size=None, maximum=32 * 1024**2):
        require(isinstance(name, str) and name and "\\" not in name, "invalid relative path")
        parts = name.split("/")
        require(all(p not in ("", ".", "..") for p in parts), "noncanonical relative path")
        path = self.root
        for part in parts:
            path /= part
            require(not path.is_symlink(), "symlink in consumed path")
        require(path.is_file() and path.resolve().is_relative_to(self.root), "missing/unsafe file")
        require(path.stat().st_size <= maximum, "consumed file exceeds bound")
        data = path.read_bytes()
        digest = sha(data)
        require(len(data) <= maximum and (size is None or len(data) == size), "byte-count mismatch")
        require(expected is None or (HASH.fullmatch(expected) and digest == expected), "hash mismatch")
        identity = {"sha256": digest, "bytes": len(data)}
        require(name not in self.consumed or self.consumed[name] == identity, "file changed during replay")
        self.consumed[name] = identity
        return data

    def json(self, name, expected=None):
        return decode(self.read(name, expected))


class Checks:
    def __init__(self):
        self.checked = 0
        self.differences = []

    def equal(self, actual, expected, label):
        """Compare explicit numeric/schema projections, retaining every mismatch."""
        if isinstance(expected, dict):
            require(isinstance(actual, dict), f"not an object: {label}")
            for key, value in expected.items():
                require(key in actual, f"missing field: {label}.{key}")
                self.equal(actual[key], value, f"{label}.{key}")
            return
        if isinstance(expected, list):
            require(isinstance(actual, list) and len(actual) == len(expected), f"list shape: {label}")
            for index, (a, b) in enumerate(zip(actual, expected, strict=True)):
                self.equal(a, b, f"{label}[{index}]")
            return
        self.checked += 1
        if isinstance(expected, float):
            try:
                okay = type(actual) in (int, float) and math.isfinite(actual) and math.isclose(
                    actual, expected, rel_tol=1e-12, abs_tol=1e-14)
            except OverflowError:
                okay = False
        else:
            okay = type(actual) is type(expected) and actual == expected
        if not okay:
            require(len(self.differences) < 10000, "more than 10000 arithmetic differences")
            self.differences.append({"field": label, "stored": actual, "independent": expected})


def distance(a, b):
    """Myers' arbitrary-width bit-vector unit-cost Levenshtein algorithm."""
    if not a:
        return len(b)
    if not b:
        return len(a)
    if len(a) > len(b):
        a, b = b, a
    width = len(a)
    masks = {}
    for index, value in enumerate(a):
        masks[value] = masks.get(value, 0) | (1 << index)
    positive, negative, score = (1 << width) - 1, 0, width
    high = 1 << (width - 1)
    for value in b:
        equal = masks.get(value, 0)
        vertical = equal | negative
        horizontal = (((equal & positive) + positive) ^ positive) | equal
        plus = negative | ~(horizontal | positive)
        minus = positive & horizontal
        score += bool(plus & high) - bool(minus & high)
        plus = (plus << 1) | 1
        minus <<= 1
        positive = minus | ~(vertical | plus)
        negative = plus & vertical
    return score


def collapse(text):
    # str.split/isspace would additionally collapse U+001C..U+001F.
    return " ".join(tokens(text))


def tokens(text):
    result, word = [], []
    for value in text:
        if value in WHITE_SPACE:
            if word:
                result.append("".join(word))
                word = []
        else:
            word.append(value)
    if word:
        result.append("".join(word))
    return result


def metric(a, b):
    return {"reference_units": len(a), "hypothesis_units": len(b), "errors": distance(a, b)}


def measure(reference, response, word_tokenizer):
    a, b = (ucd.normalize("NFC", value) for value in (reference, response))
    # Strict CER is evaluated first. Failure of any eligible metric excludes all.
    if len(a) > LIMIT or len(b) > LIMIT:
        return None
    result = {"cer": metric(a, b), "cer_whitespace_collapsed": metric(collapse(a), collapse(b)), "wer": None}
    if word_tokenizer == "whitespace":
        words_a, words_b = tokens(a), tokens(b)
        if len(words_a) > 16384 or len(words_b) > 16384:
            return None
        result["wer"] = metric(words_a, words_b)
    return result


def selected(cases, cohort, axis, level):
    return [c for c in cases if c["cohort"] == cohort and (axis == "overall" or c[axis] == level)]


def keys(cases):
    return list(dict.fromkeys((c["cohort"], axis, "all" if axis == "overall" else c[axis])
                             for c in cases for axis in AXES))


def aggregate(distances):
    ref = sum(d["reference_units"] for d in distances)
    errors = sum(d["errors"] for d in distances)
    rates = [d["errors"] / d["reference_units"] for d in distances if d["reference_units"]]
    return {"cases": len(distances), "reference_units": ref, "errors": errors,
            "hypothesis_units": sum(d["hypothesis_units"] for d in distances),
            "micro_rate": errors / ref if ref else None,
            "macro_case_rate": sum(rates) / len(rates) if rates else None,
            "empty_reference_cases": len(distances) - len(rates)}


def summaries(cases):
    out = []
    for cohort, axis, level in keys(cases):
        rows = selected(cases, cohort, axis, level)
        scored = [r for r in rows if r["outcome"] == "success"]
        out.append({"cohort": cohort, "axis": axis, "level": level,
                    "selected_cases": len(rows), "scored_cases": len(scored),
                    "service_failures": sum(r["outcome"] == "service_failure" for r in rows),
                    "metric_limits": sum(r["outcome"] == "metric_limit" for r in rows),
                    "missing_first_attempts": sum(r["outcome"] == "missing_attempt" for r in rows),
                    "document_clusters": len({r["document_cluster"] for r in rows}),
                    "scored_document_clusters": len({r["document_cluster"] for r in scored}),
                    **{m: aggregate([r["metrics"][m] for r in scored if r["metrics"][m] is not None]) for m in METRICS}})
    return out


def references(repository, cohort, root):
    base = "evaluation" + ("/laboratory-supplement" if cohort == "laboratory" else "")
    manifest_data = repository.read(base + "/corpus-frozen.json", maximum=2 * 1024**2)
    protocol = repository.json(base + "/protocol.json")
    manifest = decode(manifest_data)
    require(sha(manifest_data) == protocol["corpus_sha256"], "frozen manifest/protocol mismatch")
    require([v["id"] for v in protocol["primary_variants"]] == list(VARIANTS), "frozen variants differ")
    refs = Files(root)
    texts, counts = {}, {}
    primary = [c for c in manifest["cases"] if c["cohort"] == "primary"]
    english_letters = total_letters = 0
    for case in manifest["cases"]:
        asset = case["reference"]
        text = refs.read(asset["path"], asset["sha256"], asset["bytes"], 2 * 1024**2).decode("utf-8")
        texts[case["id"]] = text
        count = sum(ucd.category(c).startswith("L") for c in ucd.normalize("NFC", text))
        counts[case["id"]] = count
        declaration = case["english"]
        require(0 <= declaration["reference_letters"] <= count, "English count exceeds NFC letters")
        status = declaration["status"]
        require(status in ("present", "absent", "unknown") and
                (status != "absent" or declaration["reference_letters"] == 0) and
                (status != "present" or declaration["reference_letters"] > 0), "invalid English declaration")
        if case["cohort"] == "primary":
            total_letters += count
            english_letters += count if status == "unknown" else declaration["reference_letters"]
    pages = {c["original_page_id"] for c in primary}
    clusters = {c["document_cluster"] for c in primary}
    english = [c for c in primary if c["english"]["status"] != "absent"]
    english_pages = len({c["original_page_id"] for c in english})
    english_clusters = len({c["document_cluster"] for c in english})
    require(len(pages) == len(primary) == manifest["expected_primary_pages"], "primary pages/count differ")
    require(all(den > 0 and 10 * num < den for num, den in
                ((english_pages, len(pages)), (english_clusters, len(clusters)),
                 (english_letters, total_letters))), "English share is not below 10%")
    return {"manifest": manifest, "manifest_sha256": sha(manifest_data), "manifest_bytes": len(manifest_data),
            "protocol": protocol, "texts": texts, "reference_files": refs.consumed,
            "denominators": {"cases": len(texts), "primary_pages": len(pages), "primary_document_clusters": len(clusters),
                             "english_pages_conservative": english_pages, "english_clusters_conservative": english_clusters,
                             "english_reference_letters": english_letters, "reference_letters": total_letters,
                             "per_case_nfc_letters": counts}}


def successful(row):
    return row["error"] is None and type(row["status"]) is int and 200 <= row["status"] < 300


def rescore(files, run, study):
    data = files.read(run["raw_run"] + "/requests.jsonl")
    records, tail = [], 0
    # Split on LF only: bytes.splitlines also recognizes unrelated controls.
    lines = data.split(b"\n")
    if lines[-1]:
        tail = len(lines[-1])
    for line in lines[:-1]:
        require(len(line) + 1 <= 65536, "oversized journal record")
        records.append(decode(line))
    require(tail <= 65536 and records and records[0]["kind"] == "environment", "invalid journal boundary/environment")
    environment = records[0]
    require(uint(environment["schema_version"], 32, "invalid environment schema") == 1, "invalid environment schema")
    uint(environment["manifest_bytes"], 64, "invalid manifest byte count")
    uint(environment["selection_seed"], 64, "invalid selection seed")
    require(1 <= uint(environment["repetitions"], 16, "invalid repetition count") <= 20, "invalid repetition count")
    require(sum(r["kind"] == "environment" for r in records) == 1, "duplicate environment")
    expected_env = {"corpus_id": study["manifest"]["corpus_id"], "manifest_sha256": study["manifest_sha256"],
                    "manifest_bytes": study["manifest_bytes"], "selection_seed": study["manifest"]["selection_seed"],
                    "variant": run["variant"], "repetitions": study["protocol"]["execution"]["repetitions"]}
    require(all(environment[k] == v for k, v in expected_env.items()), "journal environment identity mismatch")
    budget = {k: uint(environment["budget"][k], 32 if k == "cpus" else 64, "invalid budget " + k)
              for k in ("cpus", "quota_us", "period_us", "memory_limit_bytes")}
    require(budget["cpus"] == run["cpus"] and budget["period_us"] > 0 and
            budget["quota_us"] == run["cpus"] * budget["period_us"] and budget["memory_limit_bytes"] == 3 * 1024**3,
            "journal CPU/memory identity mismatch")
    attempts, ends, warmups, resource_end = {}, [], 0, False
    previous_rep, ended = 0, set()
    cases_by_id = {c["id"]: c for c in study["manifest"]["cases"]}
    for row in records[1:]:
        kind = row["kind"]
        require(not resource_end, "records after resource_end")
        if kind in ("attempt", "warmup"):
            case_id, rep = row["case_id"], uint(row["repetition"], 16, "invalid attempt repetition")
            uint(row["cpus"], 32, "invalid attempt CPU count")
            uint(row["sequence"], 64, "invalid attempt sequence")
            uint(row["output_bytes"], 64, "invalid output byte count")
            uint(row["elapsed_us"], 128, "invalid elapsed time")
            if row["status"] is not None:
                uint(row["status"], 16, "invalid HTTP status")
            require(row["error"] is None or (isinstance(row["error"], str) and
                    len(row["error"].encode("utf-8")) <= 64), "invalid error label")
            require(case_id in cases_by_id and type(rep) is int and previous_rep <= rep <= environment["repetitions"] and rep not in ended,
                    "unknown case, duplicate or out-of-order repetition")
            case = cases_by_id[case_id]
            require(all(row[k] == case[k] for k in META if k != "original_page_id"), "attempt metadata differs")
            require(row["variant"] == run["variant"] and row["cpus"] == run["cpus"], "attempt arm differs")
            require(type(row["elapsed_us"]) is int and 0 <= row["elapsed_us"] < 1 << 128, "invalid elapsed time")
            expected_name = f"{case_id}-r{rep:02}.txt"
            require(PurePosixPath(row["output_path"]).name == expected_name, "response basename differs")
            body = files.read(run["raw_run"] + "/" + expected_name, row["output_sha256"], row["output_bytes"], 4 * 1024**2 + 1)
            text = body.decode("utf-8") if successful(row) else None
            previous_rep = rep
            if kind == "warmup":
                require(rep == 0 and row["sequence"] == 0 and warmups == 0, "invalid warmup")
                warmups += 1
                continue
            require(rep >= 1 and (case_id, rep) not in attempts and 0 <= row["sequence"] < len(cases_by_id), "duplicate/bad attempt")
            attempts[case_id, rep] = (row, text)
        elif kind == "repetition_end":
            rep = uint(row["repetition"], 16, "invalid ending repetition")
            uint(row["attempted_pages"], 64, "invalid ending count")
            uint(row["wall_us"], 128, "invalid ending wall time")
            require(rep == previous_rep and rep not in ended, "bad repetition end")
            chosen = [r for (cid, n), (r, _) in attempts.items() if n == rep]
            require((rep == 0 and warmups == 1 and row["attempted_pages"] == 1) or
                    (rep > 0 and len(chosen) == row["attempted_pages"] == len(cases_by_id) and
                     row["wall_us"] >= sum(r["elapsed_us"] for r in chosen)), "repetition end disagrees")
            ended.add(rep)
            if rep:
                ends.append(row)
        elif kind == "resource_end":
            require({k: uint(row["budget"][k], 32 if k == "cpus" else 64, "invalid final budget " + k) for k in budget} == budget, "budget changed")
            resource_end = True
        else:
            raise ValueError("unexpected journal record")
    require(not resource_end or tail == 0, "bytes after resource_end")
    out = []
    for case in study["manifest"]["cases"]:
        observed = [attempts[case["id"], r] for r in range(1, environment["repetitions"] + 1) if (case["id"], r) in attempts]
        first = attempts.get((case["id"], 1))
        successes = [r for r, _ in observed if successful(r)]
        metrics = None
        outcome = "missing_attempt" if first is None else "service_failure"
        if first is not None and successful(first[0]):
            metrics = measure(study["texts"][case["id"]], first[1], case["word_tokenizer"])
            outcome = "success" if metrics is not None else "metric_limit"
        row = {k: case[k] for k in META}
        row.update(case_id=case["id"], writer_cluster=case.get("writer_cluster"), word_tokenizer=case["word_tokenizer"],
                   input_sha256=case["input"]["sha256"], reference_sha256=case["reference"]["sha256"], outcome=outcome,
                   first_http_status=first[0]["status"] if first else None,
                   first_error=(first[0]["error"] if first[0]["error"] is not None else
                                ("" if successful(first[0]) else "non_success_http")) if first else "",
                   first_output_sha256=first[0]["output_sha256"] if first else "",
                   first_output_bytes=first[0]["output_bytes"] if first else None,
                   first_elapsed_us=first[0]["elapsed_us"] if first else None,
                   attempts=len(observed), successful_attempts=len(successes), failed_attempts=len(observed) - len(successes),
                   missing_attempts=environment["repetitions"] - len(observed),
                   successful_outputs_differing_from_first_success=sum(r["output_sha256"] != successes[0]["output_sha256"] for r in successes),
                   mixed_success_and_failure=0 < len(successes) < len(observed), metrics=metrics)
        if outcome == "metric_limit":
            row["first_error"] += "metric_codepoint_limit"
        out.append(row)
    complete = resource_end and ended == set(range(environment["repetitions"] + 1))
    return {"cases": out, "summaries": summaries(out), "journal_sha256": sha(data), "journal_bytes": len(data),
            "journal_complete": complete, "truncated_final_record_bytes": tail, "warmup_attempts": warmups,
            "nondeterministic_successful_output_cases": sum(r["successful_outputs_differing_from_first_success"] != 0 for r in out),
            "mixed_success_failure_cases": sum(r["mixed_success_and_failure"] for r in out)}, attempts, ends


def quantile(values, percentile):
    if not values:
        return None
    values = sorted(values)
    lower, remainder = divmod((len(values) - 1) * percentile, 100)
    upper = min(lower + 1, len(values) - 1)
    return float(values[lower]) + float(values[upper] - values[lower]) * remainder / 100.0


def distribution(values):
    return {"samples": len(values), "sum_elapsed_us": sum(values), "min_us": min(values, default=None),
            "p50_us": quantile(values, 50), "p95_us": quantile(values, 95), "p99_us": quantile(values, 99),
            "max_us": max(values, default=None)}


def performance(cases, attempts, ends, complete, repetitions):
    rows = [a for a, _ in attempts.values()]
    count, successes = len(rows), sum(successful(r) for r in rows)
    wall = sum(e["wall_us"] for e in ends)
    expected = len(cases) * repetitions
    cohorts = []
    for cohort in ("primary", "challenge"):
        n = sum(c["cohort"] == cohort for c in cases)
        if not n:
            continue
        chosen = [r for r in rows if r["cohort"] == cohort]
        good = [r["elapsed_us"] for r in chosen if successful(r)]
        bad = [r["elapsed_us"] for r in chosen if not successful(r)]
        cohorts.append({"cohort": cohort, "selected_cases": n, "expected_attempts": n * repetitions,
                        "observed_attempts": len(chosen), "successful_attempts": len(good), "failed_attempts": len(bad),
                        "missing_attempts": n * repetitions - len(chosen), "successful_complete_body_latency": distribution(good),
                        "failed_attempt_termination_time": distribution(bad)})
    return {"selected_cases": len(cases), "expected_repetitions": repetitions, "expected_attempts": expected,
            "observed_attempts": count, "successful_attempts": successes, "failed_attempts": count - successes,
            "missing_attempts": expected - count, "completed_repetition_wall_us": wall,
            "successful_case_responses_per_complete_workload_wall_second": successes * 1000000.0 / wall
            if complete and count == expected and len(ends) == repetitions and wall else None,
            "cohorts": cohorts, "completed_repetitions": [
                {"repetition": end["repetition"], "attempts": len(chosen),
                 "successful_attempts": sum(successful(r) for r in chosen),
                 "failed_attempts": sum(not successful(r) for r in chosen),
                 "sum_attempt_elapsed_us": sum(r["elapsed_us"] for r in chosen), "wall_us": end["wall_us"]}
                for end in ends for chosen in [[r for r in rows if r["repetition"] == end["repetition"]]]]}


class SplitMix:
    def __init__(self, seed):
        self.state = seed

    def next(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def sample(self, upper):
        require(upper > 0, "empty bootstrap population")
        threshold = (1 << 64) % upper
        while True:
            value = self.next()
            if value >= threshold:
                return value % upper


def stratum_seed(base, cohort, axis, level):
    payload = base.to_bytes(8, "little")
    for value in (cohort, axis, level):
        data = value.encode("utf-8")
        payload += len(data).to_bytes(8, "little") + data
    return int.from_bytes(hashlib.sha256(payload).digest()[:8], "little")


def estimate(clusters, seed, resamples):
    # Each tuple is (reference units, left errors, right errors, paired cases).
    eligible = [c for c in clusters if c[3]]
    reference, left, right, cases = (sum(c[n] for c in eligible) for n in range(4))
    result = {"paired_cases": cases, "document_clusters": len(eligible), "reference_units": reference,
              "left_errors": left, "right_errors": right, "left_rate": None, "right_rate": None,
              "right_minus_left": None, "ci_95_low": None, "ci_95_high": None,
              "bootstrap_samples": 0, "undefined_bootstrap_samples": 0}
    require(resamples == 0 or 1000 <= resamples <= 10000, "invalid frozen resample count")
    if not reference:
        return result
    result.update(left_rate=left / reference, right_rate=right / reference,
                  right_minus_left=(float(right) - float(left)) / reference)
    if len(eligible) < 2 or resamples == 0:
        return result
    rng, samples = SplitMix(seed), []
    for _ in range(resamples):
        denominator = numerator_left = numerator_right = 0
        for _ in eligible:
            row = eligible[rng.sample(len(eligible))]
            denominator += row[0]
            numerator_left += row[1]
            numerator_right += row[2]
        if denominator:
            samples.append((float(numerator_right) - float(numerator_left)) / denominator)
        else:
            result["undefined_bootstrap_samples"] += 1
    result["bootstrap_samples"] = resamples
    if result["undefined_bootstrap_samples"] == 0:
        samples.sort()
        for name, q in (("ci_95_low", 0.025), ("ci_95_high", 0.975)):
            position = q * (len(samples) - 1)
            lower, upper = math.floor(position), math.ceil(position)
            result[name] = samples[lower] + (samples[upper] - samples[lower]) * (position - lower)
    return result


def compare(left, right, base_seed, resamples):
    require([r["case_id"] for r in left] == [r["case_id"] for r in right], "paired case order differs")
    out, draws = [], 0
    for cohort, axis, level in keys(left):
        indices = [i for i, c in enumerate(left) if c["cohort"] == cohort and (axis == "overall" or c[axis] == level)]
        joint = [i for i in indices if left[i]["metrics"] is not None and right[i]["metrics"] is not None]
        row = {"cohort": cohort, "axis": axis, "level": level, "seed": stratum_seed(base_seed, cohort, axis, level),
               "selected_cases": len(indices), "joint_success_cases": len(joint)}
        for name, rows in (("left", left), ("right", right)):
            row[name + "_service_failures"] = sum(rows[i]["outcome"] == "service_failure" for i in indices)
            row[name + "_unscoreable"] = sum(rows[i]["outcome"] in ("metric_limit", "missing_attempt") for i in indices)
        for metric_name in METRICS:
            clusters = {}
            for index in joint:
                a, b = left[index]["metrics"][metric_name], right[index]["metrics"][metric_name]
                require((a is None) == (b is None), "WER eligibility differs")
                if a is None:
                    continue
                require(a["reference_units"] == b["reference_units"], "paired reference units differ")
                cluster = clusters.setdefault(left[index]["document_cluster"], [0, 0, 0, 0])
                for n, value in enumerate((a["reference_units"], a["errors"], b["errors"], 1)):
                    cluster[n] += value
            values = [clusters[k] for k in sorted(clusters)]
            if len(values) >= 2 and sum(c[0] for c in values):
                draws += len(values) * resamples
            row[metric_name] = estimate(values, row["seed"], resamples)
        out.append(row)
    return out, draws


def draw_upper_bound(study):
    cases = study["manifest"]["cases"]
    draws = 0
    for cohort, axis, level in keys(cases):
        rows = selected(cases, cohort, axis, level)
        for metric_name in METRICS:
            clusters = {c["document_cluster"] for c in rows
                        if metric_name != "wer" or c["word_tokenizer"] == "whitespace"}
            if len(clusters) >= 2:
                draws += len(clusters) * study["protocol"]["metrics"]["bootstrap_resamples"]
    return draws


def replay(files, studies, expected_source, checks, progress=None):
    index = files.json("study-index.json")
    require(index["source_sha"] == expected_source and index["assigned_cpu_budget"] is None, "expected combined source differs")
    require(all(type(row["cpus"]) is int for row in index["runs"] + index["comparisons"]), "invalid assigned CPU type")
    require(all(type(row["sequence"]) is int for row in index["runs"]) and
            {row["sequence"] for row in index["runs"]} == set(range(1, 25)), "invalid assigned sequence")
    expected_runs = set(itertools.product(studies, (1, 2), VARIANTS))
    require(len(index["runs"]) == 24 and {(r["cohort"], r["cpus"], r["variant"]) for r in index["runs"]} == expected_runs,
            "24-cell assignment differs")
    expected_pairs = {(c, cpu, a, b) for c in studies for cpu in (1, 2) for a, b in itertools.combinations(VARIANTS, 2)}
    require(len(index["comparisons"]) == 60 and
            {(r["cohort"], r["cpus"], r["left"], r["right"]) for r in index["comparisons"]} == expected_pairs,
            "60-pair assignment differs")
    progress = {} if progress is None else progress
    cells = [{**{k: run.get(k) for k in ("sequence", "cohort", "cpus", "variant", "status", "reason", "driver_exit", "score_exit")},
              "numeric_replay": "not_available"} for run in index["runs"]]
    pairs = [{**row, "numeric_replay": "not_available"} for row in index["comparisons"]]
    progress.update(cells=cells, comparisons=pairs, bootstrap_cluster_draws_requested=0,
                    bootstrap_scope="Every saved paired interval is recomputed with every frozen draw; no subsampling. Rejection RNG may consume additional words.")
    reconstructed = {}
    run_by_key = {(run["cohort"], run["cpus"], run["variant"]): run for run in index["runs"]}
    for run, status in zip(index["runs"], cells, strict=True):
        # Preserve failed/incomplete cells and score every available journal-backed
        # score artifact; a numeric match never promotes a failed driver to complete.
        if run.get("scores") is None:
            require(run["status"] != "completed", "completed cell missing scores")
            continue
        study = studies[run["cohort"]]
        score = files.json(run["scores"], run["sha256"])
        require(all(type(v) is int and 0 <= v <= 255 for key in ("nfc_unicode_version", "rust_unicode_version") for v in score[key]), "invalid Unicode version type")
        require(score["nfc_unicode_version"] == [16, 0, 0], "NFC Unicode version differs")
        require(score["rust_unicode_version"] == RUST_UCD, "Rust Unicode version differs from the whitespace probe")
        rebuilt, attempts, ends = rescore(files, run, study)
        checks.equal(score, {**rebuilt, "schema_version": 1, "selection_seed": study["manifest"]["selection_seed"], "manifest_sha256": study["manifest_sha256"], "manifest_bytes": study["manifest_bytes"],
                            "variant": run["variant"], "cpus": run["cpus"], "selected_accuracy_repetition": 1,
                            "metric_policy": "nfc-unicode-scalar-unit-cost-levenshtein-v1",
                            "expected_repetitions": study["protocol"]["execution"]["repetitions"],
                            "corpus_id": study["manifest"]["corpus_id"]}, f"cell{run['sequence']}.scores")
        if run.get("performance") is not None:
            saved = files.json(run["performance"], run["performance_sha256"])
            timings = performance(rebuilt["cases"], attempts, ends, rebuilt["journal_complete"], study["protocol"]["execution"]["repetitions"])
            checks.equal(saved, {**timings, "schema_version": 1, "corpus_id": study["manifest"]["corpus_id"],
                                 "manifest_sha256": study["manifest_sha256"], "manifest_bytes": study["manifest_bytes"],
                                 "variant": run["variant"], "cpus": run["cpus"], "journal_sha256": rebuilt["journal_sha256"], "journal_bytes": rebuilt["journal_bytes"],
                                 "journal_complete": rebuilt["journal_complete"], "truncated_final_record_bytes": rebuilt["truncated_final_record_bytes"],
                                 "warmup_attempts_excluded": rebuilt["warmup_attempts"]}, f"cell{run['sequence']}.performance")
        reconstructed[run["cohort"], run["cpus"], run["variant"]] = rebuilt["cases"]
        status["numeric_replay"] = "checked"
    draws = 0
    for row, status in zip(index["comparisons"], pairs, strict=True):
        if row["path"] is None:
            require(row["status"] != "completed", "completed pair missing report")
            continue
        report = files.json(row["path"], row["sha256"])
        left = reconstructed.get((row["cohort"], row["cpus"], row["left"]))
        right = reconstructed.get((row["cohort"], row["cpus"], row["right"]))
        require(left is not None and right is not None, "pair lacks independently reconstructed arms")
        protocol = studies[row["cohort"]]["protocol"]["metrics"]
        strata, work = compare(left, right, protocol["bootstrap_seed"], protocol["bootstrap_resamples"])
        side_headers = {}
        for side in ("left", "right"):
            arm = run_by_key[row["cohort"], row["cpus"], row[side]]
            side_headers[side + "_scores_sha256"] = arm["sha256"]
            side_headers[side + "_scores_bytes"] = files.consumed[arm["scores"]]["bytes"]
        checks.equal(report, {**side_headers, "schema_version": 1,
                              "metric_policy": "nfc-unicode-scalar-unit-cost-levenshtein-v1",
                              "corpus_id": studies[row["cohort"]]["manifest"]["corpus_id"],
                              "selection_seed": studies[row["cohort"]]["manifest"]["selection_seed"],
                              "manifest_bytes": studies[row["cohort"]]["manifest_bytes"], "bootstrap_seed": protocol["bootstrap_seed"], "requested_bootstrap_samples": protocol["bootstrap_resamples"],
                              "manifest_sha256": studies[row["cohort"]]["manifest_sha256"],
                              "left_variant": row["left"], "right_variant": row["right"],
                              "left_cpus": row["cpus"], "right_cpus": row["cpus"],
                              "strata": strata}, f"pair.{row['cohort']}.{row['cpus']}.{row['left']}.{row['right']}")
        draws += work
        progress["bootstrap_cluster_draws_requested"] = draws
        status["numeric_replay"] = "checked"
    return progress


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--main-references", type=Path, required=True)
    parser.add_argument("--laboratory-references", type=Path, required=True)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--source-sha", required=True, help="Expected application producer, separate from checker source")
    parser.add_argument("--output", type=Path, required=True, help="New JSON receipt; never overwrites")
    args = parser.parse_args()
    require(SOURCE.fullmatch(args.source_sha), "expected producer must be a full Git SHA")
    checks = Checks()
    result = {"schema_version": 1, "passed": False, "scope": "Independent arithmetic only; no image/signature/resource admission or new OCR.",
              "expected_application_source": args.source_sha, "python_version": sys.version, "python_executable": sys.executable,
              "python_executable_sha256": sha(Path(sys.executable).resolve().read_bytes()),
              "unicode_database_version": ucd.unidata_version, "rust_unicode_version": RUST_UCD,
              "white_space_scalars": sorted(map(ord, WHITE_SPACE)), "checker_sha256": sha(Path(__file__).read_bytes()),
              "platform": platform.platform(), "affinity": sorted(os.sched_getaffinity(0)), "failures": []}
    repository, evidence = Files(args.repository), None
    try:
        require(ucd.unidata_version == UCD, "Python NFC/category UCD must be exactly 16.0.0")
        checker_root = Path(__file__).resolve().parents[2]
        result["checker_source_sha"] = subprocess.check_output(["git", "-C", str(checker_root), "rev-parse", "HEAD"], text=True).strip()
        committed_checker = subprocess.check_output(["git", "-C", str(checker_root), "show", result["checker_source_sha"] + ":evaluation/independent/replay.py"])
        require(committed_checker == Path(__file__).read_bytes(), "checker differs from its committed producer")
        result["application_source_files"] = {}
        source_paths = ("evaluation/runner/src/metrics.rs", "evaluation/runner/src/score.rs",
                        "evaluation/runner/src/score/bootstrap.rs", "evaluation/runner/src/score/performance.rs",
                        "evaluation/runner/src/corpus.rs", "evaluation/runner/Cargo.lock")
        for name in source_paths:
            data = subprocess.check_output(["git", "-C", str(args.repository), "show", args.source_sha + ":" + name])
            result["application_source_files"][name] = {"sha256": sha(data), "bytes": len(data)}
        for cohort in ("main", "laboratory"):
            base = "evaluation" + ("/laboratory-supplement" if cohort == "laboratory" else "")
            for name in (base + "/corpus-frozen.json", base + "/protocol.json"):
                immutable = subprocess.check_output(["git", "-C", str(args.repository), "show", args.source_sha + ":" + name])
                require(repository.read(name) == immutable, "frozen input differs from expected producer Git bytes")
        studies = {cohort: references(repository, cohort, root) for cohort, root in
                   (("main", args.main_references), ("laboratory", args.laboratory_references))}
        result["references"] = {cohort: {k: value for k, value in study.items() if k in ("denominators", "reference_files", "manifest_sha256", "manifest_bytes")}
                                for cohort, study in studies.items()}
        for cohort, study in studies.items():
            bound = draw_upper_bound(study)
            result["references"][cohort]["bootstrap_cluster_draw_upper_bound_per_pair"] = bound
            result["references"][cohort]["bootstrap_cluster_draw_upper_bound_all_30_pairs"] = bound * 30
        if args.evidence:
            evidence = Files(args.evidence)
            result["replay"] = {}
            replay(evidence, studies, args.source_sha, checks, result["replay"])
        result["passed"] = not checks.differences
    except (OSError, ValueError, KeyError, TypeError, UnicodeError, OverflowError, subprocess.CalledProcessError) as error:
        result["failures"].append({"type": type(error).__name__, "message": str(error)})
    result.update(checked_scalar_fields=checks.checked, arithmetic_differences=checks.differences,
                  consumed_repository_files=repository.consumed, consumed_evidence_files=evidence.consumed if evidence else {})
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, indent=2, ensure_ascii=True, allow_nan=False)
        output.write("\n")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
