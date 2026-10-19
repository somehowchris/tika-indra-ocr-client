"""Reporting-only synthetic examples; never OCR predictions or study evidence."""
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

import figures
import study


def synthetic_study():
    result = {"source_sha": "SYNTHETIC-NOT-A-COMMIT", "index_sha256": "f" * 64,
              "statuses": study.missing_cells(), "measurements": [], "comparisons": [], "comparison_statuses": study.missing_comparisons()}
    for cohort, count in (("main", 41), ("laboratory", 2)):
        for cpus in (1, 2):
            for rank, variant in enumerate(study.VARIANTS):
                all_failed = cohort == "main" and cpus == 2 and rank == 3
                summaries = []
                for group, selected in (("primary", 33 if cohort == "main" else 2), ("challenge", 8 if cohort == "main" else 0)):
                    if not selected:
                        continue
                    failed = selected if all_failed else int(rank == 1)
                    scored = selected - failed
                    rate = None if not scored else (1.42 if rank == len(study.VARIANTS) - 1 else .02 + rank * .01)
                    summaries.append({"axis": "overall", "cohort": group, "scored_cases": scored, "selected_cases": selected,
                        "service_failures": failed, "metric_limits": 0, "scored_document_clusters": 0 if all_failed else (19 if cohort == "main" and group == "primary" else selected),
                        **{metric: {"micro_rate": None if rate is None else rate * factor,
                                    "reference_units": scored * 100, "errors": 0 if rate is None else round(rate * factor * scored * 100)}
                           for metric, factor in (("cer", 1), ("cer_whitespace_collapsed", .6))}})
                attempts = [{"case_id": f"synthetic-case-{i % count}", "cohort": "primary" if i % count < (33 if cohort == "main" else 2) else "challenge",
                             "repetition": i // count + 1, "elapsed_us": (i + rank + 1) * 1000,
                             "status": 503 if all_failed or (rank == 1 and i % count == 0) else 200,
                             "error": "http_status" if all_failed or (rank == 1 and i % count == 0) else None} for i in range(count * 3)]
                successes = sum(v["error"] is None for v in attempts)
                for summary in summaries:
                    summary.update(level="all", missing_first_attempts=0, document_clusters=19 if cohort == "main" and summary["cohort"] == "primary" else summary["selected_cases"])
                    for aggregate in (summary["cer"], summary["cer_whitespace_collapsed"]):
                        aggregate.update(cases=summary["scored_cases"], hypothesis_units=aggregate["reference_units"], empty_reference_cases=0, macro_case_rate=aggregate["micro_rate"])
                    summary["wer"] = copy.deepcopy(summary["cer"])
                originals = copy.deepcopy(summaries)
                for axis in ("language", "script", "domain", "appearance", "reference_quality"):
                    for summary in originals:
                        summaries.append({**copy.deepcopy(summary), "axis": axis, "level": "synthetic"})
                cases = []
                for i, attempt in enumerate(attempts[:count]):
                    failed = attempt["error"] is not None
                    metrics = {metric: {"reference_units": 100, "hypothesis_units": 100, "errors": i + 1} for metric in ("cer", "cer_whitespace_collapsed", "wer")}
                    if i == 0:
                        metrics["wer"] = None
                    cases.append({"case_id": attempt["case_id"], **{k: "synthetic" for k in study.METADATA},
                        "cohort": attempt["cohort"], "word_tokenizer": "none" if i == 0 else "whitespace", "outcome": "service_failure" if failed else "success",
                        "first_http_status": attempt["status"], "first_elapsed_us": attempt["elapsed_us"], "first_output_bytes": 0,
                        "first_output_sha256": "a" * 64, "input_sha256": "b" * 64, "reference_sha256": "c" * 64,
                        "metrics": None if failed else metrics})
                result["measurements"].append({"cohort": cohort, "cpus": cpus, "variant": variant,
                    "score": {"summaries": summaries, "cases": cases}, "timings": attempts,
                    "resources": {field: "{}" if field.startswith(("memory_events_", "cpu_counters_")) else 100 for field in study.RESOURCE_FIELDS},
                    "performance": {"expected_attempts": len(attempts), "successful_attempts": successes,
                        "failed_attempts": len(attempts) - successes, "completed_repetition_wall_us": 10_000_000,
                        "successful_case_responses_per_complete_workload_wall_second": successes / 10}})
                if rank:
                    for_group = []
                    for summary in summaries:
                        selected = summary["selected_cases"]
                        for_group.append({"axis": summary["axis"], "cohort": summary["cohort"], "joint_success_cases": 0 if all_failed else selected - int(rank == 1),
                            "selected_cases": selected, "left_service_failures": 0, "right_service_failures": summary["service_failures"],
                            "left_unscoreable": 0, "right_unscoreable": 0,
                            "cer": {"right_minus_left": None if all_failed else (rank - 3) / 100,
                                "ci_95_low": None if cohort == "laboratory" or all_failed else (rank - 4) / 100,
                                "ci_95_high": None if cohort == "laboratory" or all_failed else (rank - 2) / 100,
                                "paired_cases": 0 if all_failed else selected - int(rank == 1),
                                "reference_units": 0 if all_failed else selected * 100, "document_clusters": 0 if all_failed else min(19, selected),
                                "bootstrap_samples": 0 if cohort == "laboratory" or all_failed else 10000, "undefined_bootstrap_samples": 0}})
                    result["comparisons"].append({"cohort": cohort, "cpus": cpus,
                        "report": {"left_variant": study.VARIANTS[0], "right_variant": variant, "strata": for_group}})
    return result


class FigureTests(unittest.TestCase):
    def test_type7_latency_uses_all_verified_successes(self):
        value = synthetic_study()
        row = figures.tables(value)["performance"][0]
        self.assertEqual(row["p50_ms"], 62)
        self.assertAlmostEqual(row["p95_ms"], 116.9)
        self.assertEqual(row["successes"], 123)
        self.assertEqual(row["expected"], 123)
        self.assertEqual(row["throughput"], 12.3)
        self.assertEqual(row["latency_quantile_method"], "linear_type7_successful_nonwarmup_attempts")

    def test_all_failed_arm_retains_counts_and_null_quality_latency(self):
        data = figures.tables(synthetic_study())
        failed = next(r for r in data["performance"] if r["study"] == "main" and r["cpus"] == 2 and r["variant"] == study.VARIANTS[3])
        self.assertEqual((failed["successes"], failed["failures"], failed["p50_ms"], failed["throughput"]), (0, 123, None, 0))
        quality = [r for r in data["quality"] if r["study"] == "main" and r["cpus"] == 2 and r["variant"] == study.VARIANTS[3]]
        self.assertEqual(len(quality), 4)
        self.assertTrue(all(r["value"] is None and r["scored"] == 0 and r["failures"] == r["selected"] for r in quality))

    def test_resource_export_keeps_all_scopes_and_unsubmitted_values_absent(self):
        rows = figures.tables(synthetic_study())["resources"]
        self.assertEqual(len(rows), 24)
        self.assertEqual(set(rows[0]), {"study", "cpus", "variant", *study.RESOURCE_FIELDS})
        self.assertEqual(rows[0]["parent_memory_peak_observed_bytes"], 100)
        self.assertEqual(rows[0]["memory_events_after_driver"], "{}")

    def test_main_throughput_not_duplicated_per_quality_cohort(self):
        data = figures.tables(synthetic_study())
        self.assertEqual(len([r for r in data["performance"] if r["study"] == "main"]), 12)
        self.assertEqual(len([r for r in data["quality"] if r["study"] == "main"]), 48)
        self.assertEqual(len(data["attempts"]), 1548)

    def test_paired_values_keep_direction_denominators_and_absent_intervals(self):
        rows = figures.tables(synthetic_study())["paired"]
        first = rows[0]
        self.assertEqual((first["variant"], first["value"], first["low"], first["high"]), (study.VARIANTS[1], -2, -3, -1))
        self.assertEqual((first["joint"], first["paired_cases"], first["clusters"], first["reference_units"]), (32, 32, 19, 3300))
        self.assertTrue(all(r["low"] is None and r["high"] is None for r in rows if r["study"] == "laboratory"))
        self.assertFalse(any(r["variant"] == study.VARIANTS[0] for r in rows))

    def test_direct_case_and_six_axis_exports_preserve_wer_eligibility(self):
        data = figures.tables(synthetic_study())
        self.assertEqual(len(data["cases"]), 1548)
        self.assertEqual({r["axis"] for r in data["summaries"]}, {"overall", "language", "script", "domain", "appearance", "reference_quality"})
        self.assertEqual({r["metric"] for r in data["summaries"]}, {"cer", "cer_whitespace_collapsed", "wer"})
        first = next(r for r in data["cases"] if r["study"] == "main" and r["cpus"] == 1 and r["variant"] == study.VARIANTS[0] and r["metric"] == "wer")
        self.assertFalse(first["metric_eligible"])
        self.assertFalse(first["metric_available"])
        self.assertIsNone(first["rate"])
        case = next(r for r in data["cases"] if r["study"] == "main" and r["cpus"] == 1 and r["variant"] == study.VARIANTS[0] and r["case_id"] == "synthetic-case-1" and r["metric"] == "cer")
        self.assertEqual((case["reference_units"], case["errors"], case["rate"]), (100, 2, .02))

    def test_cer_is_not_capped_at_one(self):
        rows = figures.tables(synthetic_study())["quality"]
        self.assertTrue(any(r["value"] == 142 for r in rows))

    def test_missing_or_one_sided_interval_is_rejected(self):
        value = synthetic_study()
        value["comparisons"][0]["report"]["strata"][0]["cer"]["ci_95_low"] = None
        with self.assertRaises(study.AdmissionError):
            figures.tables(value)

    def test_empty_admission_exports_no_measured_numbers_or_figures(self):
        with tempfile.TemporaryDirectory() as directory:
            result = figures.render({"statuses": study.missing_cells(), "measurements": [], "comparisons": [], "comparison_statuses": study.missing_comparisons()}, directory)
            self.assertEqual(result["plots"], [])
            self.assertEqual(result["admitted_cells"], 0)
            self.assertEqual(len((Path(directory) / "comparison-statuses.csv").read_text().splitlines()), 61)
            self.assertFalse(result["synthetic"])
            self.assertEqual(len((Path(directory) / "quality.csv").read_text().splitlines()), 1)

    @unittest.skipUnless(os.environ.get("OCR_REPORT_SYNTHETIC_SMOKE"), "explicit reporting-only synthetic visual smoke")
    def test_actual_seaborn_artists_synthetic_only(self):
        destination = Path(os.environ["OCR_REPORT_SYNTHETIC_SMOKE"])
        self.assertFalse(destination.exists(), "synthetic visual output must be new")
        result = figures.render(synthetic_study(), destination, synthetic=True)
        self.assertTrue(result["synthetic"])
        self.assertEqual(len(result["plots"]), 5)
        self.assertTrue(all("SYNTHETIC SMOKE" in p["title"] for p in result["plots"]))
        self.assertEqual(sum(len(p["verified_artists"]) for p in result["plots"]), 20)
        self.assertEqual(len(list(destination.glob("*.png"))), 5)
        self.assertEqual(len(list(destination.glob("*.svg"))), 5)
        stored = json.loads((destination / "plot-validation.json").read_text())
        self.assertEqual(stored["study_index_sha256"], "f" * 64)
        self.assertEqual(stored["visual_review"], "pending")
        pairs = next(p for p in result["plots"] if p["stem"] == "main-primary-paired")
        self.assertEqual(pairs["verified_artists"][0]["intervals"][0]["low"], -3)
        quality = next(p for p in result["plots"] if p["stem"] == "main-primary-quality")
        self.assertIn(study.VARIANTS[3], quality["verified_artists"][2]["missing_numeric_rows"])
        self.assertNotIn(study.VARIANTS[3], quality["verified_artists"][0]["missing_numeric_rows"])
        self.assertGreater(quality["verified_artists"][0]["x_limits"][1], 142)
        self.assertIn("F33", quality["verified_artists"][2]["row_labels"][3])
        self.assertIn("F0", quality["verified_artists"][0]["row_labels"][3])
        self.assertTrue(all(panel["y_axis_label"] == "" for plot in result["plots"] for panel in plot["verified_artists"]))
        self.assertEqual(len({tuple(panel["x_limits"]) for panel in quality["verified_artists"]}), 1)


if __name__ == "__main__":
    unittest.main()
