"""Synthetic exports for the descriptive analysis; none is an OCR or performance measurement."""
import csv
from pathlib import Path
import tempfile
import unittest

import analysis
import figures
import study
from test_figures import synthetic_study

V0, V5 = study.VARIANTS[0], study.VARIANTS[5]


def write(directory, name, rows):
    with (Path(directory) / (name + ".csv")).open("w", newline="") as stream:
        writer = csv.DictWriter(stream, list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def case(variant, case_id, cluster, domain, metric, reference, errors, available=True, language="deu", cpus=1):
    return {"study": "main", "cpus": cpus, "variant": variant, "case_id": case_id, "document_cluster": cluster, "cohort": "primary",
            "language": language, "domain": domain, "reference_quality": "audited_pdf_text" if domain == "modern_legal" else "editorial_transcription",
            "metric": metric, "metric_available": available, "reference_units": reference if available else "", "errors": errors if available else ""}


# Strict and collapsed (reference, errors) per page and arm; L* are legal pages.
PAGES = {"L1": ("L", "modern_legal", 6000, {V0: (600, 60), V5: (500, 10)}),
         "L2": ("L", "modern_legal", 2500, {V0: (250, 25), V5: (200, 5)}),
         "O1": ("O1", "literary_prose", 3000, {V0: (900, 870), V5: (1200, 1150)}),
         "O2": ("O2", "literary_prose", 800, {V0: (400, 390), V5: (300, 290)}),
         "O3": ("O3", "poetry", 300, {V0: (150, 140), V5: (60, 50)})}


def page_rows(cpus=1):
    rows = []
    for case_id, (cluster, domain, length, errors) in PAGES.items():
        for variant, (strict, collapsed) in errors.items():
            rows.append(case(variant, case_id, cluster, domain, "cer", length, strict, cpus=cpus))
            rows.append(case(variant, case_id, cluster, domain, "cer_whitespace_collapsed", length - 100, collapsed, cpus=cpus))
    return rows


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)


class LengthTraitAndLegalTests(Fixture):
    def test_prespecified_bins_split_only_where_legal_and_other_pages_mix(self):
        write(self.directory, "cases", page_rows() + [case(V0, "O4", "O4", "poetry", "cer", 900, 90), case(V0, "O4", "O4", "poetry", "cer_whitespace_collapsed", 800, 80),
                                                      case(V5, "O4", "O4", "poetry", "cer", 900, 9, available=False)])
        rows, note = analysis.length_bins(self.directory)
        self.assertEqual([r["length bin"] for r in rows], ["<500", "500–1999", "≥2000", "≥2000 · modern_legal", "≥2000 · other domains"])
        pooled = rows[2]
        self.assertEqual((pooled["pages"], pooled["clusters"], pooled["reference codepoints"]), (3, 2, 11500))
        # Strict (600 + 250 + 900) / 11500; collapsed (60 + 25 + 870) / 11200.
        self.assertEqual(pooled["fast PSM3"], "15.22% · 8.53%")
        self.assertEqual(rows[3]["Rust Indra"], "8.24% · 0.18%")
        self.assertEqual(rows[1]["best PSM3"], "no value · no value (0/2 scored)")
        # Rust Indra's O4 attempt is unscoreable: the bin counts the page, the arm's rate does not.
        self.assertEqual((rows[1]["pages"], rows[1]["fast PSM3"], rows[1]["Rust Indra"]), (2, "28.82% · 31.33%", "37.50% · 41.43% (1/2 scored)"))
        self.assertIn("[500, 2000]", note)
        self.assertIn("<500: 1 page, 500–1999: 2 pages, ≥2000: 3 pages", note)
        self.assertIn("The longest primary page (6000 codepoints) is modern_legal.", note)
        self.assertIn("hold 63% of the primary strict reference codepoints, so the pooled bins mix", note)

    def test_traits_follow_the_manifest_and_leave_out_provenance_flags(self):
        write(self.directory, "cases", page_rows())
        manifest = {"cases": [{"id": "L1", "traits": ["running_headers", "footnotes", "training_overlap_unknown"]}, {"id": "L2", "traits": ["footnotes"]},
                              {"id": "O1", "traits": ["tables"]}, {"id": "O2", "traits": None}, {"id": "O3"}]}
        rows = analysis.traits(self.directory, manifest)
        self.assertEqual([r["trait"] for r in rows], ["tables", "footnotes", "running_headers"])
        footnotes = rows[1]
        self.assertEqual((footnotes["group"], footnotes["cohort"], footnotes["cases"], footnotes["clusters"], footnotes["domains"]),
                         ("layout", "primary", 2, 1, "modern_legal"))
        self.assertEqual((footnotes["fast PSM3"], footnotes["Rust Indra"], footnotes["best PSM3"]), ("10.00% · 1.02%", "8.24% · 0.18%", "no value · no value"))
        self.assertIn("Of the 3 trait and cohort groups, 3 come from a single document cluster and 3 from a single domain", analysis.traits_text(rows))
        self.assertEqual(analysis.traits_text([]), "")

    def test_case_aggregation_reproduces_the_exported_domain_summaries(self):
        cases = page_rows()
        write(self.directory, "cases", cases)
        summaries = []
        for variant in (V0, V5):
            for metric in ("cer", "cer_whitespace_collapsed"):
                selected = [r for r in cases if (r["variant"], r["metric"], r["domain"]) == (variant, metric, "modern_legal")]
                summaries.append({"study": "main", "cpus": 1, "variant": variant, "cohort": "primary", "axis": "domain", "level": "modern_legal", "metric": metric,
                                  "errors": sum(r["errors"] for r in selected), "reference_units": sum(r["reference_units"] for r in selected), "micro_rate": .1})
        write(self.directory, "summaries", summaries)
        self.assertEqual(analysis.reproduces_summaries(self.directory), 4)
        summaries[0]["errors"] += 1
        write(self.directory, "summaries", summaries)
        with self.assertRaisesRegex(study.AdmissionError, "case aggregation differs"):
            analysis.reproduces_summaries(self.directory)

    def test_legal_whitespace_share_and_collapsed_errors_by_language(self):
        cases = page_rows() + [case(V0, "L3", "L3", "modern_legal", "cer_whitespace_collapsed", 1000, 40, language="ell")]
        write(self.directory, "cases", cases)
        summary = lambda variant, metric, axis, level, errors, reference, rate: {
            "study": "main", "cpus": 1, "variant": variant, "cohort": "primary", "axis": axis, "level": level, "metric": metric,
            "selected_cases": 2, "document_clusters": 1, "errors": errors, "reference_units": reference, "micro_rate": rate}
        write(self.directory, "summaries", [
            summary(V0, "cer", "domain", "modern_legal", 850, 8500, .1), summary(V0, "cer_whitespace_collapsed", "domain", "modern_legal", 85, 8300, .01),
            summary(V0, "wer", "domain", "modern_legal", 50, 1000, .05), summary(V0, "cer", "overall", "all", 2300, 12600, .18),
            summary(V5, "cer", "domain", "modern_legal", 700, 8500, .08), summary(V5, "cer_whitespace_collapsed", "domain", "modern_legal", 15, 8300, ""),
            summary(V5, "wer", "domain", "modern_legal", 9, 1000, .009), summary(V5, "cer", "overall", "all", 2260, 12600, .18)])
        rows, by_language, references = analysis.legal(self.directory)
        self.assertEqual(len(rows), 1)  # an absent collapsed rate leaves the arm out, never at zero
        self.assertEqual((rows[0]["strict errors removed by collapsing"], rows[0]["share of headline strict CER (pp)"]), (90.0, 6.07))
        self.assertEqual((by_language[("1", "deu", V0)], by_language[("1", "ell", V0)], references), (85, 40, ["audited_pdf_text"]))
        text = analysis.legal_text(rows, by_language, references)
        self.assertIn("The 2 modern legal pages (1 cluster) have audited_pdf_text references", text)
        self.assertIn("removes 90% of each arm's strict errors there, which is 6.07 pp", text)
        self.assertIn("deu no value for Python Indra, 15 for Rust Indra against 85 for Tesseract; ell no value for either Indra arm against 40", text)


class ResultTextTests(Fixture):
    def setUp(self):
        super().setUp()
        figures.export(figures.tables(synthetic_study()), self.directory)

    def test_quality_reads_each_budget_and_keeps_failed_arms_absent(self):
        text = analysis.quality(self.directory)
        self.assertIn("At 1 CPU, on the 33 main primary pages (19 document clusters): strict CER is 6.00% for Python Indra, 142.00% for Rust Indra "
                      "against 2.00–5.00% for the Tesseract settings; collapsed CER is 3.60% for Python Indra, 85.20% for Rust Indra against 1.20–3.00%", text)
        self.assertIn("1 first attempt failed across the arms", text)
        # Best PSM11 failed every 2-CPU attempt: absent from the range, counted as failures.
        self.assertIn("At 2 CPUs, on the 33 main primary pages (19 document clusters): strict CER is 6.00% for Python Indra, 142.00% for Rust Indra "
                      "against 2.00–4.00%", text)
        self.assertIn("34 first attempts failed across the arms", text)

    def test_identical_arms_and_budgets_read_once(self):
        rows = analysis.read(self.directory, "quality")
        for row in rows:
            if row["variant"] == study.VARIANTS[4]:
                row["value"] = next(r["value"] for r in rows if (r["study"], r["cohort"], r["cpus"], r["metric"], r["variant"]) ==
                                    (row["study"], row["cohort"], row["cpus"], row["metric"], V5))
            if row["cpus"] == "2":
                row.update(next({"value": r["value"], "failures": r["failures"]} for r in rows if (r["study"], r["cohort"], r["cpus"], r["metric"], r["variant"]) ==
                                (row["study"], row["cohort"], "1", row["metric"], row["variant"])))
        write(self.directory, "quality", rows)
        text = analysis.quality(self.directory)
        self.assertTrue(text.startswith("At both CPU budgets, on the 33 main primary pages"))
        self.assertIn("strict CER is 142.00% for either Indra arm", text)

    def test_paired_counts_intervals_that_exclude_zero_and_marks_missing_estimates(self):
        text = analysis.paired(self.directory)
        # Synthetic differences are right minus left minus 3 pp with ±1 pp intervals.
        self.assertIn("+1.00 pp (+0.00 to +2.00) for Python Indra, +2.00 pp (+1.00 to +3.00) for Rust Indra against fast PSM3", text)
        self.assertIn("no paired estimate for either Indra arm against best PSM11", text)
        self.assertIn("Of the 16 intervals (2 Indra arms, 4 Tesseract settings, 2 budgets), 3 exclude zero: Python Indra against best PSM11, "
                      "Rust Indra against fast PSM3.", text)

    def test_worse_strata_name_every_level_with_its_size(self):
        strata = analysis.strata_versus_tesseract(self.directory)
        self.assertEqual({(r["cpus"], r["axis"]) for r in strata}, {(c, a) for c in (1, 2) for a in analysis.STRATA})
        first = strata[0]
        self.assertEqual((first["Indra strict"], first["best Tesseract"], first["its strict"], first["cases"]), ("6.00% / 142.00%", "fast PSM3", "2.00%", 33))
        text = analysis.worse_strata_text(strata)
        self.assertTrue(text.startswith("At both CPU budgets, both Indra arms have a higher strict CER than the best Tesseract setting on 5 main primary strata"))
        self.assertNotIn("Handwritten", text)
        self.assertEqual(analysis.worse_strata_text([]), "No main primary stratum has a higher strict CER for both Indra arms than for the best Tesseract setting.")

    def test_discussion_states_directions_from_the_rows(self):
        self.assertEqual(analysis.strata_versus_tesseract(self.directory, worse=False), [])
        paragraphs = analysis.discussion(self.directory, synthetic_study()["hosts"])
        self.assertTrue(paragraphs[0].startswith("The Indra arms do not have a lower strict and collapsed CER than every Tesseract setting"))
        # Synthetic Python Indra minus setting l is (1 - l) ± 1 pp, Rust Indra (2 - l) ± 1 pp.
        self.assertIn("The paired intervals of Python Indra include zero against fast PSM3, best PSM3, best PSM6, so the data do not show a difference "
                      "from those settings, and exclude zero against best PSM11 without a multiplicity adjustment.", paragraphs[0])
        self.assertIn("The paired intervals of Rust Indra include zero against best PSM3, best PSM6, best PSM11", paragraphs[0])
        self.assertIn("and better than every Tesseract setting on no stratum. 0 of these 5 strata come from one or two document clusters", paragraphs[1])
        self.assertEqual(paragraphs[2], "On the two laboratory crops Indra's strict CER is higher than every Tesseract setting's; with two crops this is "
                                        "a description of those crops, not of laboratory documents.")
        self.assertIn("ran on different hosts with different CPU models and AVX-512F support, so no 1- versus 2-CPU comparison", paragraphs[3])
        self.assertEqual(len(paragraphs), 4)  # no modern legal pages in the synthetic export

    def test_laboratory_crops_stay_descriptive(self):
        text = analysis.laboratory(self.directory)
        self.assertIn("on the two laboratory crops, strict CER is 6.00% for Python Indra, 142.00% for Rust Indra against 2.00–5.00% for Tesseract", text)
        # Best PSM3 failed crop 0, so only crop 1 has every arm's errors.
        self.assertIn("the largest per-crop difference is on synthetic-case-1, strict errors 2 for either Indra arm against 2 for fast PSM3", text)
        self.assertIn("zero bootstrap resamples", text)

    def test_cpu_cost_per_success_names_whether_one_host_ran_both_budgets(self):
        resources = [{"study": "main", "cpus": c, "variant": v, "warmup_workload_cpu_usage_usec": u}
                     for c, v, u in ((1, V0, 246_000_000), (2, V0, 492_000_000), (1, V5, 123_000_000), (2, V5, 123_000_000))]
        performance = [{"study": "main", "cpus": r["cpus"], "variant": r["variant"], "successes": 123, "throughput": .5} for r in resources]
        write(self.directory, "resources", resources)
        write(self.directory, "performance", performance)
        rows = analysis.cpu_cost(self.directory)
        self.assertEqual([r["CPU s per success"] for r in rows], [2.0, 1.0, 4.0, 1.0])
        hosts = synthetic_study()["hosts"]
        text = analysis.cpu_cost_text(rows, hosts)
        self.assertIn("fast PSM3 2.000 s at 1 CPU and 4.000 s at 2 CPUs; Rust Indra 1.000 s at 1 CPU and 1.000 s at 2 CPUs", text)
        self.assertIn("uses 1.00–2.00× the CPU time", text)
        self.assertIn("The budgets ran on different hosts (1 CPU: Synthetic CPU A, with AVX-512F; 2 CPU: Synthetic CPU B, no AVX-512F)", text)
        one = [{**h, "host": "synthetic-a", "kernel": "6.0", "cpu_model": "Synthetic CPU A"} for h in hosts]
        self.assertIn("Both budgets ran on one host", analysis.cpu_cost_text(rows, one))

    def test_dispatch_names_each_budget_and_what_is_not_logged(self):
        hosts = synthetic_study()["hosts"]
        text = analysis.dispatch_text(hosts)
        self.assertIn("at 1 CPU its CPUID check finds AVX-512F and OpenCV's feature line reads ` SSE SSE2 SSE3 *AVX2 *AVX512-SKX `; "
                      "at 2 CPUs its CPUID check does not find AVX-512F and OpenCV's feature line reads ` SSE SSE2 SSE3 *AVX2 *AVX512-SKX? `.", text)
        self.assertIn("ONNX Runtime does not log its kernel choice", text)
        self.assertEqual(analysis.dispatch_text([{**h, "rust_isa_avx512f": None, "rust_opencv_features": None} for h in hosts]),
                         "No completed Rust run of an admitted budget logged a cpu_dispatch record.")
        self.assertEqual(analysis.code("a `b` $x$"), "`` a `b` $x$ ``")

    def test_repetition_spread_and_rust_python_ratio(self):
        text = analysis.repetition_text(self.directory)
        self.assertTrue(text.startswith("Post-hoc descriptive addendum"))
        self.assertIn("At 1 CPU, per-repetition throughput spans fast PSM3 11.08–13.67", text)
        self.assertIn("Rust/Python throughput ratio is 1.000 pooled and 1.00 per repetition", text)
        self.assertIn("best PSM11 0.00", text)

    def test_every_figure_gets_a_reading_computed_from_its_rows(self):
        readings = analysis.readings(self.directory)
        self.assertEqual(set(readings), set(figures.STEMS))
        self.assertEqual(readings["main-primary-quality"], "At 1 CPU the lowest strict CER belongs to fast PSM3 and the lowest collapsed CER to fast PSM3.")
        self.assertEqual(readings["main-primary-paired"], "Against fast PSM3 at 1 CPU, the intervals of best PSM6, best PSM11, Python Indra cross zero.")
        self.assertTrue(readings["laboratory-quality"].startswith("On the 2 laboratory crops at 1 CPU the lowest strict CER belongs to fast PSM3"))
        self.assertIn("has the highest throughput at 1 CPU (12.300 responses/s, median 62 ms); the widest per-repetition throughput whisker is", readings["main-workload-performance"])
        self.assertIn("(33 cases; 142.00% strict, 85.20% collapsed)", readings["main-primary-strata"])


if __name__ == "__main__":
    unittest.main()
