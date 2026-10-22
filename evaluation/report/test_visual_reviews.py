"""Committed figures must still be the exact bytes their visual-review receipts approved.

Dependency-free: no Seaborn, Quarto or network. A regenerated PNG/SVG resets
approval to pending by construction, because its hash no longer matches.
"""
import hashlib
import json
from pathlib import Path
import unittest

import study

ROOT = study.ROOT
FIGURES = ROOT / "evaluation/figures"
HISTORICAL_REVIEW = ROOT / "evaluation/report/historical-visual-review.json"
ABLATION_REVIEW = FIGURES / "native-ablations-visual-review.json"
FROZEN_STUDY_REVIEW = FIGURES / "frozen-study-visual-review.json"
FROZEN_STUDY_STEMS = ("main-primary-quality", "main-primary-paired", "main-workload-performance",
                      "main-challenge-quality", "laboratory-quality", "main-primary-strata")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class VisualReviewTests(unittest.TestCase):
    def test_historical_review_binds_both_committed_pngs(self):
        review = json.loads(HISTORICAL_REVIEW.read_text())
        self.assertEqual(review["schema_version"], 1)
        self.assertEqual(set(review["files"]), {"evaluation/figures/signed-baseline-p95.png", "evaluation/figures/signed-baseline-throughput.png"})
        for relative, digest in review["files"].items():
            with self.subTest(path=relative):
                path = ROOT / relative
                self.assertTrue(path.is_file() and not path.is_symlink())
                self.assertEqual(sha256(path), digest)

    def test_ablation_review_binds_source_data_figures_and_bytes(self):
        review = json.loads(ABLATION_REVIEW.read_text())
        self.assertTrue(review["accepted"])
        self.assertEqual(len(review["files"]), 5)
        for entry in review["files"]:
            with self.subTest(path=entry["path"]):
                path = ROOT / entry["path"]
                self.assertTrue(path.is_file() and not path.is_symlink())
                self.assertEqual(path.stat().st_size, entry["bytes"])
                self.assertEqual(sha256(path), entry["sha256"])

    def test_frozen_study_review_binds_the_rendered_figures_to_the_sealed_measurement(self):
        review = json.loads(FROZEN_STUDY_REVIEW.read_text())
        self.assertTrue(review["accepted"])
        self.assertEqual(review["measurement"]["release_run_id"], "39810108196")
        self.assertEqual(review["measurement"]["source_sha"], "92ba8d549e32f6a12b9e6154b238114cf754c10b")
        self.assertRegex(review["measurement"]["study_index_sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual(review["render"]["report_source_sha"][:7], review["render"]["report_source_sha"][:7])
        paths = {entry["path"] for entry in review["files"]}
        expected = {f"evaluation/figures/frozen-study-{stem}.{suffix}" for stem in FROZEN_STUDY_STEMS for suffix in ("png", "svg")}
        expected |= {"evaluation/figures/frozen-study-plot-validation.json", "evaluation/report/figures.py"}
        self.assertEqual(paths, expected)
        for entry in review["files"]:
            with self.subTest(path=entry["path"]):
                path = ROOT / entry["path"]
                self.assertTrue(path.is_file() and not path.is_symlink())
                self.assertEqual(path.stat().st_size, entry["bytes"])
                self.assertEqual(sha256(path), entry["sha256"])
        validation = json.loads((FIGURES / "frozen-study-plot-validation.json").read_text())
        self.assertFalse(validation["synthetic"])
        self.assertEqual(validation["measurement_source_sha"], review["measurement"]["source_sha"])
        self.assertEqual(validation["study_index_sha256"], review["measurement"]["study_index_sha256"])
        self.assertEqual({plot["stem"] for plot in validation["plots"]}, set(FROZEN_STUDY_STEMS))
        strata = next(plot for plot in validation["plots"] if plot["stem"] == "main-primary-strata")
        self.assertGreaterEqual(strata["level_label_gap_inches"], strata["widest_level_label_inches"])

    def test_every_committed_png_has_a_review_receipt(self):
        historical = set(json.loads(HISTORICAL_REVIEW.read_text())["files"])
        ablation = {entry["path"] for entry in json.loads(ABLATION_REVIEW.read_text())["files"]}
        frozen = {entry["path"] for entry in json.loads(FROZEN_STUDY_REVIEW.read_text())["files"]}
        committed = {p.relative_to(ROOT).as_posix() for p in FIGURES.glob("*.png")}
        self.assertTrue(committed)
        self.assertEqual(committed - historical - ablation - frozen, set())

    def test_signed_baseline_validation_receipt_names_the_reviewed_stems(self):
        validation = json.loads((FIGURES / "signed-baseline-plot-validation.json").read_text())
        stems = {plot["file_stem"] for plot in validation["plots"]}
        self.assertEqual(stems, {"signed-baseline-throughput", "signed-baseline-p95"})
        self.assertTrue(all(plot["zero_baseline"] for plot in validation["plots"]))
        self.assertEqual(validation["visual_review"], "pending")


if __name__ == "__main__":
    unittest.main()
