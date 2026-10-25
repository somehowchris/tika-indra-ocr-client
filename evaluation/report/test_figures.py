"""Reporting-only synthetic examples; never OCR predictions or study evidence."""
import copy
import importlib.util
import itertools
import json
import os
from pathlib import Path
import tempfile
import unittest

import figures
import study


def synthetic_study():
    result = {**study.unsubmitted(), "source_sha": "SYNTHETIC-NOT-A-COMMIT", "index_sha256": "f" * 64,
              "hosts": [{"cpus": 1, "host": "synthetic-a", "kernel": "6.0", "cpu_model": "Synthetic CPU A", "avx512f": True, "selected_cpus": "0",
                         "physical_cores": 1, "core_ids": "0", "thread_siblings": "0-1", "process_allowed_cpus": "0-3"},
                        {"cpus": 2, "host": "synthetic-b", "kernel": "6.0", "cpu_model": "Synthetic CPU B", "avx512f": False, "selected_cpus": "0,1",
                         "physical_cores": 1, "core_ids": "0,0", "thread_siblings": "0-1;0-1", "process_allowed_cpus": "0-3"}]}
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
                        "successful_case_responses_per_complete_workload_wall_second": successes / 10,
                        "completed_repetitions": [{"repetition": repetition, "wall_us": wall,
                            "attempts": count, "successful_attempts": sum(a["error"] is None for a in attempts if a["repetition"] == repetition),
                            "failed_attempts": sum(a["error"] is not None for a in attempts if a["repetition"] == repetition),
                            "sum_attempt_elapsed_us": sum(a["elapsed_us"] for a in attempts if a["repetition"] == repetition)}
                            for repetition, wall in ((1, 3_700_000), (2, 3_000_000), (3, 3_300_000))]}})
                result.setdefault("_summaries", {})[(cohort, cpus, rank)] = summaries
    for cohort in ("main", "laboratory"):
        for cpus in (1, 2):
            for (left_rank, left), (right_rank, right) in itertools.combinations(enumerate(study.VARIANTS), 2):
                all_failed = cohort == "main" and cpus == 2 and 3 in (left_rank, right_rank)
                for_group = []
                for summary in result["_summaries"][(cohort, cpus, right_rank)]:
                    selected = summary["selected_cases"]
                    failing = int(1 in (left_rank, right_rank))
                    def estimate(factor):
                        value = None if all_failed else (right_rank - left_rank - 3) / 100 * factor
                        return {"right_minus_left": value,
                                "ci_95_low": None if cohort == "laboratory" or all_failed else value - .01,
                                "ci_95_high": None if cohort == "laboratory" or all_failed else value + .01,
                                "paired_cases": 0 if all_failed else selected - failing,
                                "reference_units": 0 if all_failed else selected * 100, "document_clusters": 0 if all_failed else min(19, selected),
                                "bootstrap_samples": 0 if cohort == "laboratory" or all_failed else 10000, "undefined_bootstrap_samples": 0}
                    for_group.append({"axis": summary["axis"], "cohort": summary["cohort"], "joint_success_cases": 0 if all_failed else selected - failing,
                        "selected_cases": selected, "left_service_failures": int(left_rank == 1), "right_service_failures": int(right_rank == 1),
                        "left_unscoreable": 0, "right_unscoreable": 0,
                        "cer": estimate(1), "cer_whitespace_collapsed": estimate(.6), "wer": estimate(1.3)})
                result["comparisons"].append({"cohort": cohort, "cpus": cpus,
                    "report": {"left_variant": left, "right_variant": right, "strata": for_group}})
    del result["_summaries"]
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
        self.assertEqual((first["left"], first["variant"], first["metric"]), (study.VARIANTS[0], study.VARIANTS[1], "cer"))
        self.assertEqual((first["value"], first["low"], first["high"]), (-2, -3, -1))
        self.assertEqual((first["joint"], first["paired_cases"], first["clusters"], first["reference_units"]), (32, 32, 19, 3300))
        self.assertTrue(all(r["low"] is None and r["high"] is None for r in rows if r["study"] == "laboratory"))
        self.assertFalse(any(r["variant"] == study.VARIANTS[0] for r in rows))

    def test_paired_export_keeps_all_sixty_pairs_and_three_metrics(self):
        rows = figures.tables(synthetic_study())["paired"]
        # 15 pairs × 2 CPU × (2 main overall cohorts + 1 laboratory cohort) × 3 metrics.
        self.assertEqual(len(rows), 270)
        self.assertEqual({(r["study"], r["cpus"], r["left"], r["variant"]) for r in rows},
                         {(s, c, l, r) for s in ("main", "laboratory") for c in (1, 2) for l, r in itertools.combinations(study.VARIANTS, 2)})
        self.assertEqual({r["metric"] for r in rows}, set(figures.PAIRED_METRICS))
        self.assertTrue(all(study.VARIANTS.index(r["left"]) < study.VARIANTS.index(r["variant"]) for r in rows))
        collapsed = next(r for r in rows if r["left"] == study.VARIANTS[1] and r["variant"] == study.VARIANTS[2] and r["metric"] == "cer_whitespace_collapsed" and r["study"] == "main" and r["cohort"] == "primary" and r["cpus"] == 1)
        self.assertAlmostEqual(collapsed["value"], -1.2)
        self.assertEqual((collapsed["left_failures"], collapsed["right_failures"], collapsed["joint"]), (1, 0, 32))
        failed = [r for r in rows if r["study"] == "main" and r["cpus"] == 2 and study.VARIANTS[3] in (r["left"], r["variant"])]
        self.assertEqual(len(failed), 5 * 2 * 3)
        self.assertTrue(all(r["value"] is None and r["low"] is None and r["joint"] == 0 for r in failed))

    def test_plotted_pairs_are_the_prespecified_baseline_strict_cer_subset(self):
        rows = figures.tables(synthetic_study())["paired"]
        plotted = figures.baseline_pairs(rows)
        self.assertEqual(len(plotted), 10)
        self.assertTrue(all(r["left"] == study.VARIANTS[0] and r["metric"] == "cer" and r["study"] == "main" and r["cohort"] == "primary" for r in plotted))
        self.assertEqual(sorted({r["variant"] for r in plotted}), sorted(study.VARIANTS[1:]))

    def test_strata_rows_cover_three_axes_for_every_arm_and_budget(self):
        data = figures.tables(synthetic_study())
        rows = figures.strata_rows(data["summaries"])
        self.assertEqual(len(rows), 3 * 6 * 2)
        self.assertEqual({r["axis"] for r in rows}, set(figures.STRATA_AXES))
        self.assertTrue(all(r["level"] == "synthetic" and r["selected"] == 33 for r in rows))
        failed = [r for r in rows if r["cpus"] == 2 and r["variant"] == study.VARIANTS[3]]
        self.assertEqual(len(failed), 3)
        self.assertTrue(all(r["value"] is None and r["scored"] == 0 and r["failures"] == 33 for r in failed))
        self.assertTrue(any(r["value"] == 142 for r in rows))
        duplicate = rows + rows[:1]
        data["summaries"].append(next(s for s in data["summaries"] if s["axis"] == "appearance"))
        with self.assertRaises(study.AdmissionError):
            figures.strata_rows(data["summaries"])
        self.assertEqual(len(duplicate), 37)

    def test_strata_rows_pair_each_strict_row_with_its_collapsed_row(self):
        data = figures.tables(synthetic_study())
        row = next(r for r in figures.strata_rows(data["summaries"]) if r["cpus"] == 1 and r["variant"] == study.VARIANTS[0] and r["axis"] == "domain")
        self.assertEqual((row["value"], row["collapsed"]), (2, 1.2))
        self.assertEqual((row["collapsed_reference_units"], row["collapsed_errors"]), (3300, 40))
        data["summaries"].remove(next(s for s in data["summaries"] if s["axis"] == "domain" and s["metric"] == "cer_whitespace_collapsed"))
        with self.assertRaisesRegex(study.AdmissionError, "unpaired stratum row"):
            figures.strata_rows(data["summaries"])

    def test_repetition_export_keeps_each_repetitions_throughput_and_median(self):
        rows = figures.tables(synthetic_study())["performance-repetitions"]
        self.assertEqual(len(rows), 24 * 3)
        first = [r for r in rows if (r["study"], r["cpus"], r["variant"]) == ("main", 1, study.VARIANTS[0])]
        self.assertEqual([r["repetition"] for r in first], [1, 2, 3])
        self.assertEqual([(r["successes"], r["wall_us"]) for r in first], [(41, 3_700_000), (41, 3_000_000), (41, 3_300_000)])
        self.assertEqual([r["throughput"] for r in first], [41_000_000 / 3_700_000, 41_000_000 / 3_000_000, 41_000_000 / 3_300_000])
        # Repetition r holds attempts 41(r-1) .. 41r-1 at (i + 1) ms: medians 21, 62, 103 ms.
        self.assertEqual([r["p50_ms"] for r in first], [21, 62, 103])
        failed = [r for r in rows if (r["study"], r["cpus"], r["variant"]) == ("main", 2, study.VARIANTS[3])]
        self.assertTrue(all(r["successes"] == 0 and r["throughput"] == 0 and r["p50_ms"] is None for r in failed))
        self.assertTrue(all(tuple(r) == figures.TABLES["performance-repetitions"] for r in rows))

    def test_repetitions_that_do_not_add_up_are_rejected(self):
        value = synthetic_study()
        value["measurements"][0]["performance"]["completed_repetitions"][0]["successful_attempts"] -= 1
        with self.assertRaisesRegex(study.AdmissionError, "repetition totals differ"):
            figures.tables(value)

    def test_host_note_names_each_budget_and_whether_one_host_ran_both(self):
        hosts = synthetic_study()["hosts"]
        self.assertEqual(figures.tables(synthetic_study())["hosts"], hosts)
        self.assertEqual(figures.host_note(hosts), [
            "1 CPU: Synthetic CPU A, with AVX-512F · logical CPU 0 on 1 physical core (sibling lists 0-1) · host synthetic-a, kernel 6.0",
            "2 CPU: Synthetic CPU B, no AVX-512F · logical CPUs 0,1 on 1 physical core (sibling lists 0-1;0-1) · host synthetic-b, kernel 6.0",
            "The budgets ran on different hosts, so a per-arm 1-versus-2 CPU difference mixes host and budget."])
        one = [{**host, "host": "synthetic-a", "cpu_model": "Synthetic CPU A", "avx512f": True} for host in hosts]
        self.assertEqual(figures.host_note(one)[-1], "Both budgets ran on one host.")
        self.assertIn("on unknown physical cores", figures.host_note([{**hosts[1], "physical_cores": None}])[0])
        self.assertEqual(figures.host_note([]), [])

    def test_unordered_pair_is_rejected(self):
        value = synthetic_study()
        report = value["comparisons"][0]["report"]
        report["left_variant"], report["right_variant"] = report["right_variant"], report["left_variant"]
        with self.assertRaises(study.AdmissionError):
            figures.tables(value)

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
            result = figures.render(study.unsubmitted(), directory)
            self.assertEqual(result["plots"], [])
            self.assertEqual(result["admitted_cells"], 0)
            self.assertEqual(len((Path(directory) / "comparison-statuses.csv").read_text().splitlines()), 61)
            self.assertFalse(result["synthetic"])
            self.assertEqual(len((Path(directory) / "quality.csv").read_text().splitlines()), 1)

    @unittest.skipUnless(importlib.util.find_spec("seaborn"), "the plotting environment is not installed")
    def test_long_stratum_levels_widen_the_gap_instead_of_overlapping(self):
        # The measured domain levels are long; the first hosted render drew them
        # into the neighbouring panel. Every gap must hold the widest label.
        compact_study, wide_study = synthetic_study(), synthetic_study()
        for measurement in wide_study["measurements"]:
            for summary in measurement["score"]["summaries"]:
                if summary["axis"] == "domain":
                    summary["level"] = "historical_merchandise_catalogue_synthetic"
        with tempfile.TemporaryDirectory() as directory:
            compact = figures.plot_strata(figures.strata_rows(figures.tables(compact_study)["summaries"]), "main-primary-strata", Path(directory), True)
            wide = figures.plot_strata(figures.strata_rows(figures.tables(wide_study)["summaries"]), "main-primary-strata", Path(directory), True)
        for result in (compact, wide):
            self.assertGreaterEqual(result["level_label_gap_inches"], result["widest_level_label_inches"])
        self.assertGreater(wide["level_label_gap_inches"], compact["level_label_gap_inches"])
        domain = next(panel for panel in wide["verified_artists"] if panel["panel"] == "domain")
        self.assertEqual(domain["row_labels"], ["historical_merchandise_catalogue_synthetic · n33"])

    @unittest.skipUnless(importlib.util.find_spec("seaborn"), "the plotting environment is not installed")
    def test_equal_values_of_six_arms_stay_one_mark_apart_in_a_nine_level_panel(self):
        # The measured domain axis has nine levels; at the first offsets equal
        # values of several arms hid one another there.
        rows = figures.strata_rows(figures.tables(synthetic_study())["summaries"])
        rows = [r for r in rows if r["axis"] != "domain"] + [
            {**r, "level": f"domain-{n}", "value": 50.0, "collapsed": 40.0} for r in rows if r["axis"] == "domain" for n in range(9)]
        with tempfile.TemporaryDirectory() as directory:
            result = figures.plot_strata(rows, figures.STEMS[5], Path(directory), True)
        self.assertGreaterEqual(result["arm_offset_points"], result["mark_diameter_points"])
        domain = next(panel for panel in result["verified_artists"] if (panel["cpus"], panel["panel"]) == (1, "domain"))
        self.assertEqual(len(domain["row_labels"]), 9)
        self.assertEqual(len({m["y"] for m in domain["marks"]}), 9 * 6)
        self.assertEqual(sorted((m["variant"], m["y"]) for m in domain["collapsed_marks"]), sorted((m["variant"], m["y"]) for m in domain["marks"]))

    @unittest.skipUnless(importlib.util.find_spec("seaborn"), "the plotting environment is not installed")
    def test_drawn_figures_are_closed(self):
        # An open figure is printed by the notebook as "<Figure ... with 0 Axes>".
        import matplotlib.pyplot as plt
        data = figures.tables(synthetic_study())
        with tempfile.TemporaryDirectory() as directory:
            figures.plot([r for r in data["performance"] if r["study"] == "main"], [("throughput", "Throughput")],
                         "closed", "note", figures.STEMS[2], Path(directory), "performance", True)
            figures.plot_strata(figures.strata_rows(data["summaries"]), figures.STEMS[5], Path(directory), True)
        self.assertEqual(plt.get_fignums(), [])

    @unittest.skipUnless(os.environ.get("OCR_REPORT_SYNTHETIC_SMOKE"), "explicit reporting-only synthetic visual smoke")
    def test_actual_seaborn_artists_synthetic_only(self):
        destination = Path(os.environ["OCR_REPORT_SYNTHETIC_SMOKE"])
        self.assertFalse(destination.exists(), "synthetic visual output must be new")
        result = figures.render(synthetic_study(), destination, synthetic=True)
        self.assertTrue(result["synthetic"])
        self.assertEqual(len(result["plots"]), 6)
        self.assertEqual([p["stem"] for p in result["plots"]], [figures.STEMS[i] for i in (0, 3, 4, 1, 2, 5)])
        self.assertTrue(all("SYNTHETIC SMOKE" in p["title"] for p in result["plots"]))
        self.assertEqual(sum(len(p["verified_artists"]) for p in result["plots"]), 26)
        self.assertEqual(len(list(destination.glob("*.png"))), 6)
        self.assertEqual(len(list(destination.glob("*.svg"))), 6)
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
        self.assertEqual(len(pairs["verified_artists"][0]["marks"]), 5)
        workload = next(p for p in result["plots"] if p["stem"] == "main-workload-performance")
        ranges = {(panel["cpus"], panel["panel"]): panel["intervals"] for panel in workload["verified_artists"]}
        self.assertEqual(ranges[(1, "Successful case responses / workload s")][0], {"variant": study.VARIANTS[0], "low": 41_000_000 / 3_700_000, "high": 41_000_000 / 3_000_000})
        self.assertEqual(ranges[(1, "Successful complete-body p50 (ms)")][0], {"variant": study.VARIANTS[0], "low": 21, "high": 103})
        self.assertEqual(ranges[(1, "Successful complete-body p95 (ms)")], [])
        self.assertEqual(len(ranges[(2, "Successful complete-body p50 (ms)")]), 5)
        strata = next(p for p in result["plots"] if p["stem"] == "main-primary-strata")
        self.assertEqual([(panel["cpus"], panel["panel"]) for panel in strata["verified_artists"]],
                         [(cpus, axis) for cpus in (1, 2) for axis in figures.STRATA_AXES])
        self.assertEqual(sum(len(panel["marks"]) for panel in strata["verified_artists"]), 36 - 3)
        self.assertEqual(sum(len(panel["collapsed_marks"]) for panel in strata["verified_artists"]), 36 - 3)
        self.assertGreaterEqual(strata["arm_offset_points"], strata["mark_diameter_points"])
        self.assertEqual([m["variant"] for panel in strata["verified_artists"] if panel["cpus"] == 2 for m in panel["missing_numeric_rows"]], [study.VARIANTS[3]] * 3)
        self.assertTrue(all(panel["row_labels"] == ["synthetic · n33"] for panel in strata["verified_artists"]))
        self.assertTrue(all(len(panel["coverage"]) == 6 for panel in strata["verified_artists"]))
        self.assertEqual(len({tuple(panel["x_limits"]) for panel in strata["verified_artists"]}), 1)


if __name__ == "__main__":
    unittest.main()
