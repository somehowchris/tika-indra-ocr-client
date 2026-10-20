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

    def test_every_committed_png_has_a_review_receipt(self):
        historical = set(json.loads(HISTORICAL_REVIEW.read_text())["files"])
        ablation = {entry["path"] for entry in json.loads(ABLATION_REVIEW.read_text())["files"]}
        committed = {p.relative_to(ROOT).as_posix() for p in FIGURES.glob("*.png")}
        self.assertTrue(committed)
        self.assertEqual(committed - historical - ablation, set())

    def test_signed_baseline_validation_receipt_names_the_reviewed_stems(self):
        validation = json.loads((FIGURES / "signed-baseline-plot-validation.json").read_text())
        stems = {plot["file_stem"] for plot in validation["plots"]}
        self.assertEqual(stems, {"signed-baseline-throughput", "signed-baseline-p95"})
        self.assertTrue(all(plot["zero_baseline"] for plot in validation["plots"]))
        self.assertEqual(validation["visual_review"], "pending")


if __name__ == "__main__":
    unittest.main()
