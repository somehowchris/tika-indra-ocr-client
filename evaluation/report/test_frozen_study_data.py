"""The committed frozen-study data must be the exact bytes the render receipt bound.

Dependency-free: no Seaborn, Quarto or network. The chain starts at the committed
visual-review receipt, which records the hosted render receipt's SHA-256; the render
receipt binds every exported CSV and the combined study index.
"""
import hashlib
import json
from pathlib import Path
import unittest

import study

ROOT = study.ROOT
DATA = ROOT / "evaluation/results/frozen-study-20261022"
FIGURES = ROOT / "evaluation/figures"
CSVS = ("attempts.csv", "cases.csv", "comparison-statuses.csv", "paired.csv",
        "performance.csv", "quality.csv", "resources.csv", "summaries.csv")
BOUND = (*CSVS, "render-receipt.json", "study-index.json")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class FrozenStudyDataTests(unittest.TestCase):
    def setUp(self):
        self.review = json.loads((FIGURES / "frozen-study-visual-review.json").read_text())
        self.receipt = json.loads((DATA / "render-receipt.json").read_text())

    def test_render_receipt_is_the_one_the_visual_review_bound(self):
        # A container reproduction records the hosted render it reproduces.
        render = self.review["render"]
        hosted = render.get("reproduces", render)
        self.assertEqual(hosted["run_id"], "39837656304")
        self.assertEqual(sha256(DATA / "render-receipt.json"), hosted["render_receipt_sha256"])
        self.assertEqual(self.receipt["source_sha"], self.review["measurement"]["source_sha"])
        self.assertEqual(self.receipt["index_sha256"], self.review["measurement"]["study_index_sha256"])
        self.assertEqual((self.receipt["admitted_cells"], self.receipt["admitted_comparisons"]), (24, 60))

    def test_every_csv_has_the_receipt_and_plot_validation_bytes(self):
        validation = json.loads((FIGURES / "frozen-study-plot-validation.json").read_text())
        for name in CSVS:
            with self.subTest(name=name):
                path = DATA / name
                self.assertTrue(path.is_file() and not path.is_symlink())
                actual = {"sha256": sha256(path), "bytes": path.stat().st_size}
                self.assertEqual(actual, self.receipt["files"][f"evaluation/report/study-data/{name}"])
                self.assertEqual(actual, validation["files"][name])

    def test_study_index_is_the_measured_index_the_receipt_bound(self):
        path = DATA / "study-index.json"
        self.assertEqual(sha256(path), self.receipt["index_sha256"])
        index = json.loads(path.read_text())
        self.assertEqual(index["status"], "measured")
        self.assertEqual(index["source_sha"], self.receipt["source_sha"])
        self.assertEqual(index["provenance"]["release_run_id"], self.review["measurement"]["release_run_id"])
        self.assertEqual((len(index["runs"]), len(index["comparisons"])), (24, 60))

    def test_checksums_list_exactly_the_bound_files(self):
        listed = {}
        for line in (DATA / "SHA256SUMS").read_text().splitlines():
            digest, name = line.split("  ", 1)
            listed[name] = digest
        self.assertEqual(set(listed), set(BOUND))
        for name, digest in listed.items():
            with self.subTest(name=name):
                self.assertEqual(sha256(DATA / name), digest)
        present = {p.name for p in DATA.iterdir()}
        self.assertEqual(present - set(BOUND), {".gitattributes", "README.md", "SHA256SUMS"})


if __name__ == "__main__":
    unittest.main()
