"""Synthetic exports for the descriptive analysis; none is an OCR or performance measurement."""
import csv
from pathlib import Path
import tempfile
import unittest

import analysis
import study

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



if __name__ == "__main__":
    unittest.main()
