"""No model inference: tests for path relocation and immutable replay admission."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("replay", Path(__file__).with_name("replay.py"))
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
REPO = Path(__file__).resolve().parents[2]


class ReplayTests(unittest.TestCase):
    def test_preserved_records_match_harness_hashes(self):
        r.verify_preserved(REPO)

    def test_all_frozen_paths_have_known_relocation(self):
        cases = r.read(REPO / r.FROZEN / "selection-plan.json")["cases"]
        self.assertEqual(len(cases), 14)
        resolved = [r.locate(c, Path("/new-repo"), Path("/new-corpus")) for c in cases]
        self.assertEqual(sum(p is None for p in resolved), 1)
        self.assertEqual([c["id"] for c, p in zip(cases, resolved) if p is None], ["rotated-greek"])

    def test_unknown_path_rejected(self):
        with self.assertRaises(ValueError):
            r.locate({"id": "unknown", "path": "/unrecognized/file.png"}, REPO, REPO)

    def test_exclusive_json_and_nonfinite_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "receipt.json"
            r.write(path, {"status": "retained"})
            with self.assertRaises(FileExistsError):
                r.write(path, {"status": "overwritten"})
            self.assertEqual(r.read(path), {"status": "retained"})
            with self.assertRaises(ValueError):
                r.write(Path(tmp) / "nan.json", {"value": float("nan")})

    def test_input_byte_mismatch_and_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "image.png"
            path.write_bytes(b"original")
            case = {"id": "test", "input_bytes": 8, "input_sha256": r.sha(path)}
            r.verify_input(path, case)
            link = path.with_name("link.png")
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                r.verify_input(link, case)
            path.write_bytes(b"modified")
            with self.assertRaises(ValueError):
                r.verify_input(path, case)

    def test_mutated_adapter_rejected_before_predictions(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            r.write(root / "amendment.json", {"adapter_sha256": "0" * 64})
            with self.assertRaisesRegex(ValueError, "identity changed"):
                r.admission(SimpleNamespace(repo=REPO, output=root))


if __name__ == "__main__":
    unittest.main()
