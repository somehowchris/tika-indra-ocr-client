"""Synthetic admission fixtures. None is an OCR or performance measurement."""
from __future__ import annotations

import copy
import csv
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest

import study

SOURCE = "a" * 40


def write_json(root, path, value):
    target = root / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(value, allow_nan=False) + "\n")
    return hashlib.sha256(target.read_bytes()).hexdigest()


def inventory(root):
    paths = sorted(p for p in root.rglob("*") if p.is_file() and p != root / "SHA256SUMS")
    (root / "SHA256SUMS").write_text("".join(f"{study.digest(p.read_bytes())}  {p.relative_to(root).as_posix()}\n" for p in paths))


def fixture(root):
    freezes = study.frozen_studies()
    studies = []
    for cohort, frozen in freezes.items():
        base = "evaluation" if cohort == "main" else "evaluation/laboratory-supplement"
        entry = {"cohort": cohort, "selection_seed": frozen["manifest"]["selection_seed"],
                 "bootstrap_seed": frozen["protocol"]["metrics"]["bootstrap_seed"],
                 "bootstrap_resamples": frozen["protocol"]["metrics"]["bootstrap_resamples"]}
        for name, filename in [("manifest", "corpus-frozen.json"), ("protocol", "protocol.json")]:
            path = "provenance/source/" + base + "/" + filename
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((study.ROOT / base / filename).read_bytes())
            entry[name], entry[name + "_sha256"] = path, study.digest(target.read_bytes())
        studies.append(entry)
    runs = []
    for number, row in enumerate(study.missing_cells(), 1):
        runs.append({**row, "sequence": number, "status": "failed", "driver_exit": None,
                     "score_exit": None, "image_binding_exit": None,
                     **{k: None for k in ["scores", "sha256", "performance", "performance_sha256", "attempts", "attempts_sha256",
                                          "driver_result", "driver_result_sha256", "raw_run", "parent_before", "parent_after", "parent_counters_after"]}})
    comparisons = [{"cohort": c, "cpus": n, "left": l, "right": r, "status": "skipped",
                    "path": None, "sha256": None, "exit_code": None}
                   for c, n in study.itertools.product(freezes, (1, 2)) for l, r in study.itertools.combinations(study.VARIANTS, 2)]
    index = {"schema_version": 1, "source_sha": SOURCE, "assigned_cpu_budget": None,
             "assigned_budget_complete": False, "status": "partial", "studies": studies,
             "manifest_sha256": freezes["main"]["manifest_sha256"], "protocol_sha256": freezes["main"]["protocol_sha256"],
             "runs": runs, "comparisons": comparisons,
             "provenance": {"repository": "somehowchris/tika-indra-ocr-client", "release_run_id": "123", "release_run_attempt": "1", "shards": []}}
    write_json(root, "study-index.json", index)
    for cpu in (1, 2):
        prefix = f"shards/cpu{cpu}/ocr-evaluation-results/"
        p = {k: index["provenance"][k] for k in ["repository", "release_run_id", "release_run_attempt"]}
        for name in ["candidates", "bundle", "evaluator", "evaluate_script", "schedule"]:
            p[name] = f"provenance/{name}.json"
            value = {"fixture": "not measurement"}
            if name == "candidates":
                value = {**p, "source_sha": SOURCE, "images": [{"role": role, "digest": "sha256:" + "b"*64, "reference": "ghcr.io/somehowchris/tika-indra-" + role + "@sha256:" + "b"*64} for role in ["native", "gateway", "bridge", "tika", "tesseract", "backend"]]}
                value = {k: value[k] for k in ["repository", "release_run_id", "release_run_attempt", "source_sha", "images"]}
            p[name + "_sha256"] = write_json(root, prefix + p[name], value)
        shard = {"source_sha": SOURCE, "assigned_cpu_budget": cpu, "provenance": p,
                 "runs": copy.deepcopy(runs)}
        relative = prefix + "study-index.json"
        sha = write_json(root, relative, shard)
        index["provenance"]["shards"].append({"cpus": cpu, "status": "admitted", "index": relative,
                                               "index_sha256": sha, "assigned_budget_complete": False, "provenance": p})
    write_json(root, "study-index.json", index)
    inventory(root)
    return index


def resource_snapshot(usage, current, peak, events_max):
    return {"cpu_usage_usec": usage, "cpu_stat": f"usage_usec {usage}\nuser_usec {usage*3//5}\nsystem_usec {usage*2//5}\nnr_periods {usage//10}\n",
            "memory_current_bytes": current, "memory_peak_bytes": peak,
            "memory_events": f"low 0\nhigh 0\nmax {events_max}\noom 0\noom_kill 0\nfuture_event 3\n",
            "descendants": 1, "dying_descendants": 0}


def measured_fixture(root):
    frozen = study.frozen_studies()["laboratory"]
    cases = frozen["manifest"]["cases"]
    variant = study.VARIANTS[0]
    budget = {"cpus": 1, "quota_us": 100000, "period_us": 100000, "memory_limit_bytes": 3 * 1024**3,
              "parent_path": "/sys/fs/cgroup/synthetic-owned", **resource_snapshot(1000, 8000, 10000, 0)}
    environment = {"kind": "environment", "variant": variant, "corpus_id": frozen["manifest"]["corpus_id"],
                   "manifest_sha256": frozen["manifest_sha256"], "manifest_bytes": frozen["manifest_bytes"],
                   "selection_seed": frozen["manifest"]["selection_seed"], "repetitions": 3, "budget": budget}
    body = b"synthetic fixture, not an OCR output"
    body_path = root / "driver/run/output.txt"
    body_path.parent.mkdir(parents=True)
    body_path.write_bytes(body)
    records = [environment, {"kind": "warmup"}]
    attempt_rows = []
    scores = []
    for repetition in (1, 2, 3):
        for sequence, case in enumerate(cases):
            a = {"kind": "attempt", "variant": variant, "cpus": 1, "repetition": repetition,
                 "sequence": sequence, "case_id": case["id"], **{k: case[k] for k in study.METADATA},
                 "elapsed_us": 100, "status": 200, "error": None, "output_path": "output.txt",
                 "output_sha256": study.digest(body), "output_bytes": len(body)}
            records.append(a)
            row = {k: a.get(k) for k in study.CSV_FIELDS}
            row.update(http_status=200, error="", successful="true")
            attempt_rows.append(row)
            if repetition == 1:
                scores.append({"case_id": case["id"], **{k: case[k] for k in study.METADATA},
                               "word_tokenizer": case["word_tokenizer"], "input_sha256": case["input"]["sha256"],
                               "reference_sha256": case["reference"]["sha256"], "first_output_sha256": study.digest(body),
                               "first_output_bytes": len(body), "first_elapsed_us": 100, "first_http_status": 200,
                               "metrics": {m: {"reference_units": 10, "hypothesis_units": 10, "errors": 0} for m in ("cer", "cer_whitespace_collapsed", "wer")}, "outcome": "success"})
        records.append({"kind": "repetition_end", "repetition": repetition, "attempted_pages": len(cases), "wall_us": 1000})
    records.append({"kind": "resource_end", "budget": {**budget, **resource_snapshot(7000, 18000, 20000, 2)}})
    journal = ("".join(json.dumps(r) + "\n" for r in records)).encode()
    (root / "driver/run/requests.jsonl").write_bytes(journal)
    shared = {"schema_version": 1, "variant": variant, "cpus": 1, "manifest_sha256": frozen["manifest_sha256"],
              "manifest_bytes": frozen["manifest_bytes"], "corpus_id": frozen["manifest"]["corpus_id"],
              "journal_complete": True, "truncated_final_record_bytes": 0, "expected_repetitions": 3,
              "journal_sha256": study.digest(journal), "journal_bytes": len(journal)}
    score = {**shared, "selected_accuracy_repetition": 1, "selection_seed": frozen["manifest"]["selection_seed"],
             "warmup_attempts": 1, "cases": scores}
    summaries = []
    for cohort, axis, level in sorted({(c["cohort"], a, "all" if a == "overall" else c[a]) for c in scores for a in ("overall", "language", "script", "domain", "appearance", "reference_quality")}):
        selected = [c for c in scores if c["cohort"] == cohort and (axis == "overall" or c[axis] == level)]
        summaries.append({"cohort": cohort, "axis": axis, "level": level, "selected_cases": len(selected), "scored_cases": len(selected),
                          "service_failures": 0, "metric_limits": 0, "missing_first_attempts": 0,
                          "document_clusters": len({c["document_cluster"] for c in selected}), "scored_document_clusters": len({c["document_cluster"] for c in selected}),
                          **{m: {"cases": len(selected), "reference_units": 10*len(selected), "hypothesis_units": 10*len(selected), "errors": 0,
                                 "empty_reference_cases": 0, "micro_rate": 0.0, "macro_case_rate": 0.0} for m in ("cer", "cer_whitespace_collapsed", "wer")}})
    score["summaries"] = summaries
    performance = {**shared, "warmup_attempts_excluded": 1, "selected_cases": len(cases), "expected_attempts": len(cases)*3,
                   "observed_attempts": len(cases)*3, "missing_attempts": 0, "successful_attempts": len(cases)*3,
                   "failed_attempts": 0, "completed_repetition_wall_us": 3000,
                   "successful_case_responses_per_complete_workload_wall_second": len(cases)*1000,
                   "completed_repetitions": [{"repetition": i, "wall_us": 1000, "attempts": len(cases),
                                               "successful_attempts": len(cases), "failed_attempts": 0,
                                               "sum_attempt_elapsed_us": 100*len(cases)} for i in (1,2,3)]}
    run = {"cohort": "laboratory", "variant": variant, "cpus": 1, "raw_run": "driver/run"}
    for name, value, hash_key in [("scores", score, "sha256"), ("performance", performance, "performance_sha256"),
                                  ("driver_result", {"complete": True, "label": variant, "execution_error": None,
                                                     "integrity_error": None, "cleanup_errors": []}, "driver_result_sha256")]:
        run[name] = name + ".json"
        run[hash_key] = write_json(root, run[name], value)
    stream = io.StringIO()
    writer = csv.DictWriter(stream, study.CSV_FIELDS)
    writer.writeheader()
    writer.writerows(attempt_rows)
    (root / "attempts.csv").write_text(stream.getvalue())
    run.update(attempts="attempts.csv", attempts_sha256=study.digest((root / "attempts.csv").read_bytes()))
    parent = {"device": 1, "inode": 2, "memory_max": 3 * 1024**3, "memory_swap_max": 0, "cpu_max": "100000 100000\n", "path": "/sys/fs/cgroup/synthetic-owned"}
    for field in ("parent_before", "parent_after"):
        run[field] = "driver/" + field.replace("_", "-") + ".json"
        write_json(root, run[field], parent)
    run["parent_counters_after"] = "driver/parent-counters-after.json"
    write_json(root, run["parent_counters_after"], resource_snapshot(8000, 500, 20000, 2))
    write_json(root, "driver/parent-freshness.json", resource_snapshot(100, 500, 1000, 0))
    write_json(root, "driver/readiness.json", {"aggregate_start_to_ready_ms": 2500, "backend_poll_ms": 200, "gateway_poll_ms": 300})
    inventory(root)
    return run, frozen


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.index = fixture(self.root)

    def tearDown(self):
        self.temporary.cleanup()

    def save(self):
        write_json(self.root, "study-index.json", self.index)
        inventory(self.root)

    def rejected(self):
        with self.assertRaises(study.AdmissionError):
            study.load(self.root, SOURCE)

    def test_valid_partial_retains_all_cells_without_measurements(self):
        report = study.load(self.root, SOURCE)
        self.assertEqual(len(report["statuses"]), 24)
        self.assertEqual(report["measurements"], [])
        self.assertEqual(report["comparisons"], [])
        self.assertEqual(report["comparison_statuses"], [{k: row.get(k) for k in study.COMPARISON_FIELDS} for row in self.index["comparisons"]])

    def test_no_artifact_has_24_unsubmitted_cells(self):
        self.assertEqual(len(study.missing_cells()), 24)
        self.assertTrue(all(row["status"] == "unsubmitted" for row in study.missing_cells()))

    def test_failed_comparison_retains_reason_exit_and_bound_artifact(self):
        row = self.index["comparisons"][0]
        path = self.root / "comparison-error.txt"
        path.write_text("Synthetic command failure, not a study result.\n")
        row.update(status="failed", reason="synthetic helper failed", exit_code=17,
                   path=path.name, sha256=study.digest(path.read_bytes()))
        self.save()
        report = study.load(self.root, SOURCE)
        self.assertEqual(report["comparison_statuses"][0], row)
        self.assertEqual(report["comparisons"], [])

    def test_no_artifact_has_60_unsubmitted_comparisons(self):
        rows = study.missing_comparisons()
        self.assertEqual(len(rows), 60)
        self.assertEqual(len({(r["cohort"], r["cpus"], r["left"], r["right"]) for r in rows}), 60)
        self.assertTrue(all(r["status"] == "unsubmitted" and r["path"] is None and r["sha256"] is None for r in rows))

    def test_inventory_changed_bytes(self):
        (self.root / "study-index.json").write_text("{}")
        self.rejected()

    def test_extra_extraction_file(self):
        (self.root / "leftover").write_text("unverified")
        self.rejected()

    def test_missing_inventory_file(self):
        (self.root / "study-index.json").unlink()
        self.rejected()

    def test_symlink_file(self):
        (self.root / "escape").symlink_to("/etc/passwd")
        self.rejected()

    def test_symlink_directory(self):
        (self.root / "escape").symlink_to("/etc", target_is_directory=True)
        self.rejected()

    def test_duplicate_inventory_path(self):
        p = self.root / "SHA256SUMS"
        p.write_text(p.read_text() + p.read_text().splitlines()[0] + "\n")
        self.rejected()

    def test_duplicate_json_key(self):
        p = self.root / "study-index.json"
        p.write_text(p.read_text().replace('"schema_version": 1', '"schema_version": 1, "schema_version": 1'))
        inventory(self.root)
        self.rejected()

    def test_nan(self):
        p = self.root / "study-index.json"
        p.write_text(p.read_text().replace('"schema_version": 1', '"schema_version": NaN'))
        inventory(self.root)
        self.rejected()

    def test_source_mismatch(self):
        self.index["source_sha"] = "b" * 40
        self.save()
        self.rejected()

    def test_invalid_expected_source(self):
        with self.assertRaises(study.AdmissionError):
            study.load(self.root, "main")

    def test_shard_index_not_combined(self):
        self.index["assigned_cpu_budget"] = 1
        self.save()
        self.rejected()

    def test_missing_study(self):
        self.index["studies"].pop()
        self.save()
        self.rejected()

    def test_changed_frozen_hash(self):
        self.index["studies"][0]["manifest_sha256"] = "0" * 64
        self.save()
        self.rejected()

    def test_changed_seed(self):
        self.index["studies"][0]["bootstrap_seed"] += 1
        self.save()
        self.rejected()

    def test_laboratory_intervals_requested(self):
        self.index["studies"][1]["bootstrap_resamples"] = 10000
        self.save()
        self.rejected()

    def test_duplicate_cell(self):
        self.index["runs"][-1] = self.index["runs"][0]
        self.save()
        self.rejected()

    def test_missing_cell(self):
        self.index["runs"].pop()
        self.save()
        self.rejected()

    def test_unknown_variant(self):
        self.index["runs"][0]["variant"] = "best-after-looking"
        self.save()
        self.rejected()

    def test_duplicate_sequence(self):
        self.index["runs"][0]["sequence"] = self.index["runs"][1]["sequence"]
        self.save()
        self.rejected()

    def test_missing_cpu_shard(self):
        self.index["provenance"]["shards"].pop()
        self.save()
        self.rejected()

    def test_duplicate_cpu_shard(self):
        self.index["provenance"]["shards"][1] = self.index["provenance"]["shards"][0]
        self.save()
        self.rejected()

    def test_shard_provenance_changed(self):
        self.index["provenance"]["shards"][0]["provenance"]["release_run_id"] = "999"
        self.save()
        self.rejected()

    def test_combined_run_changed(self):
        self.index["runs"][0]["status"] = "completed"
        self.save()
        self.rejected()

    def test_completed_in_unadmitted_shard(self):
        self.index["provenance"]["shards"][0]["status"] = "rejected"
        self.index["runs"][0]["status"] = "completed"
        self.save()
        self.rejected()

    def test_missing_comparison(self):
        self.index["comparisons"].pop()
        self.save()
        self.rejected()

    def test_duplicate_comparison(self):
        self.index["comparisons"][-1] = self.index["comparisons"][0]
        self.save()
        self.rejected()

    def test_comparison_pairs_incomplete_run(self):
        self.index["comparisons"][0].update(status="completed", exit_code=0)
        self.save()
        self.rejected()

    def test_measured_requires_all_cells(self):
        self.index["status"] = "measured"
        self.save()
        self.rejected()


    def test_completed_run_actual_journal_csv_schema(self):
        measured = self.root / "measured"
        measured.mkdir()
        run, frozen = measured_fixture(measured)
        accepted = study.validate_run(study.Evidence(measured), run, frozen)
        self.assertEqual(accepted["performance"]["successful_attempts"], 6)

    def measured_rejected(self, filename, mutate):
        measured = self.root / "measured"
        measured.mkdir()
        run, frozen = measured_fixture(measured)
        data = json.loads((measured / filename).read_text())
        mutate(data)
        sha = write_json(measured, filename, data)
        if filename == "performance.json":
            run["performance_sha256"] = sha
        elif filename == "scores.json":
            run["sha256"] = sha
        inventory(measured)
        with self.assertRaises(study.AdmissionError):
            study.validate_run(study.Evidence(measured), run, frozen)

    def test_throughput_wrong_denominator(self):
        self.measured_rejected("performance.json", lambda p: p.update(successful_case_responses_per_complete_workload_wall_second=9))

    def test_false_complete_journal(self):
        self.measured_rejected("scores.json", lambda p: p.update(journal_complete=False))

    def test_wrong_first_attempt(self):
        self.measured_rejected("scores.json", lambda p: p["cases"][0].update(first_output_sha256="0"*64))

    def test_wrong_case_metadata(self):
        self.measured_rejected("scores.json", lambda p: p["cases"][0].update(language="eng"))

    def test_changed_cgroup_quota(self):
        self.measured_rejected("driver/parent-after.json", lambda p: p.update(cpu_max="200000 100000"))

    def test_performance_count_mismatch(self):
        self.measured_rejected("performance.json", lambda p: p.update(failed_attempts=1))

    def test_wrong_repetition_wall_time(self):
        self.measured_rejected("performance.json", lambda p: p["completed_repetitions"][0].update(wall_us=999))

    def test_boolean_cpu_identity(self):
        self.index["runs"][0]["cpus"] = True
        self.save()
        self.rejected()

    def test_missing_metric_distance(self):
        self.measured_rejected("scores.json", lambda p: p["cases"][0].update(metrics={}))

    def test_impossible_distance_counts(self):
        self.measured_rejected("scores.json", lambda p: p["cases"][0]["metrics"]["cer"].update(errors=99))

    def test_wrong_derived_rate(self):
        self.measured_rejected("scores.json", lambda p: p["summaries"][0]["cer"].update(micro_rate=0.5))

    def test_absolute_path(self):
        with self.assertRaises(study.AdmissionError):
            study.Evidence(self.root).path("/etc/passwd")

    def test_parent_traversal(self):
        with self.assertRaises(study.AdmissionError):
            study.Evidence(self.root).path("../outside")

    def test_dot_path(self):
        with self.assertRaises(study.AdmissionError):
            study.Evidence(self.root).path("provenance/./source")


class ResourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.run, self.frozen = measured_fixture(self.root)

    def validate(self, mutate_records=None, mutate_parent=None):
        records = [json.loads(line) for line in (self.root / "driver/run/requests.jsonl").read_bytes().splitlines()]
        parent = json.loads((self.root / self.run["parent_before"]).read_bytes())
        if mutate_records:
            mutate_records(records)
        if mutate_parent:
            mutate_parent(parent)
        return study.validate_resources(study.Evidence(self.root), self.run, records, parent)

    def update(self, filename, mutate):
        data = json.loads((self.root / filename).read_bytes())
        mutate(data)
        write_json(self.root, filename, data)
        inventory(self.root)  # Rehashed semantic negative, not an inventory failure.

    def test_existing_windows_preserve_nonzero_events_and_lifetime_peaks(self):
        row = self.validate()
        self.assertEqual(row["warmup_workload_cpu_usage_usec"], 6000)
        self.assertEqual(row["warmup_workload_cpu_user_usec"], 3600)
        self.assertEqual(row["warmup_workload_cpu_system_usec"], 2400)
        self.assertEqual(row["cpu_since_admission_usage_usec"], 7900)
        self.assertEqual(row["parent_memory_peak_observed_bytes"], 20000)
        self.assertEqual(row["admission_memory_peak_bytes"], 1000)  # Never subtract it from peak.
        self.assertEqual(json.loads(row["memory_events_warmup_workload_delta"])["max"], 2)
        self.assertEqual(json.loads(row["memory_events_after_driver"])["future_event"], 3)
        self.assertEqual(row["startup_ready_ms"], 2500)
        self.assertEqual(set(row), set(study.RESOURCE_FIELDS))

    def test_missing_duplicate_malformed_or_unterminated_counters(self):
        for field, value in (("cpu_stat", "user_usec 1\nsystem_usec 1\n"),
                             ("cpu_stat", "usage_usec 1\nusage_usec 1\nuser_usec 1\nsystem_usec 1\n"),
                             ("cpu_stat", "usage_usec -1\nuser_usec 1\nsystem_usec 1\n"),
                             ("cpu_stat", "usage_usec 1000\nuser_usec 600\nsystem_usec 400\njunk"),
                             ("memory_events", "low 0\nhigh 0\nmax 0\noom 0\n")):
            with self.subTest(field=field, value=value):
                with self.assertRaises(study.AdmissionError):
                    self.validate(lambda rows: rows[0]["budget"].update({field: value}))

    def test_decreasing_cpu_events_or_changed_counter_sets(self):
        for field, old, new in (("cpu_stat", "usage_usec 7000", "usage_usec 999"),
                                ("memory_events", "max 2", "max 0"),
                                ("memory_events", "future_event 3\n", "")):
            def mutate(rows):
                rows[-1]["budget"][field] = rows[-1]["budget"][field].replace(old, new)
                if field == "memory_events" and old == "max 2":
                    rows[0]["budget"][field] = rows[0]["budget"][field].replace("max 0", "max 1")
            with self.subTest(field=field, old=old):
                with self.assertRaises(study.AdmissionError):
                    self.validate(mutate)

    def test_final_raw_cpu_disagreement_and_peak_decrease(self):
        self.update("driver/parent-counters-after.json", lambda row: row.update(cpu_usage_usec=8001))
        with self.assertRaisesRegex(study.AdmissionError, "raw CPU"):
            self.validate()
        self.update("driver/parent-counters-after.json", lambda row: row.update(cpu_usage_usec=8000, memory_peak_bytes=19999))
        with self.assertRaisesRegex(study.AdmissionError, "peak decreased"):
            self.validate()

    def test_budget_types_canonical_record_and_parent_identity(self):
        for key, value in (("cpus", True), ("period_us", 100000.0), ("quota_us", 2**65),
                           ("memory_limit_bytes", 3*1024**3-1), ("parent_path", "/sys/fs/cgroup/foreign")):
            with self.subTest(key=key):
                with self.assertRaises(study.AdmissionError):
                    self.validate(lambda rows: rows[0]["budget"].update({key: value}))
        for value in ("100000 100000\njunk", "100000 100000", "+100000 100000\n", "max 100000\n"):
            with self.subTest(value=value):
                with self.assertRaises(study.AdmissionError):
                    self.validate(mutate_parent=lambda parent: parent.update(cpu_max=value))

    def test_wrong_sibling_resource_path_and_readiness_arithmetic(self):
        original = self.run["parent_counters_after"]
        self.run["parent_counters_after"] = "foreign/parent-counters-after.json"
        with self.assertRaisesRegex(study.AdmissionError, "different driver"):
            self.validate()
        self.run["parent_counters_after"] = original
        self.update("driver/readiness.json", lambda row: row.update(aggregate_start_to_ready_ms=100))
        with self.assertRaisesRegex(study.AdmissionError, "readiness windows"):
            self.validate()

    def test_current_can_decrease_and_missing_optional_journal_peak_is_explicit(self):
        row = self.validate(lambda records: records[0]["budget"].update(memory_peak_bytes=None))
        self.assertIsNone(row["before_warmup_memory_peak_bytes"])
        self.assertGreater(row["after_workload_memory_current_bytes"], row["after_driver_memory_current_bytes"])


if __name__ == "__main__":
    unittest.main()
