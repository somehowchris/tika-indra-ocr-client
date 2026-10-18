"""Verify compact replay receipts against the preserved historical protocol."""
from pathlib import Path
import hashlib
import json
import unittest

ROOT = Path(__file__).resolve().parent
EVIDENCE = ROOT / "evidence/2026-10-18"
HISTORICAL = ROOT.parents[1] / "runtime/crates/indra-engine/tests/evidence/2026-10-18-parity"


def read(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class EvidenceTests(unittest.TestCase):
    def test_rules_and_cases_are_unchanged(self):
        original = read(HISTORICAL / "protocol.json")
        fresh = read(EVIDENCE / "comparison-protocol.json")
        for key in ("settings", "rules", "process_timeout_seconds", "reference_assets", "comparison_harness_sha256"):
            self.assertEqual(original[key], fresh[key], key)
        old_cases = read(HISTORICAL / "selection-plan.json")["cases"]
        new_cases = read(EVIDENCE / "selection-plan.json")["cases"]
        self.assertEqual(len(new_cases), 14)
        for old, new in zip(old_cases, new_cases, strict=True):
            self.assertEqual({k: v for k, v in old.items() if k != "path"}, {k: v for k, v in new.items() if k != "path"})

    def test_all_actual_results_and_tensor_comparisons_pass(self):
        results = read(EVIDENCE / "native/results.json")
        self.assertEqual(len(results), 14)
        self.assertEqual(sum(r["compared_tensors"] for r in results), 850)
        for result in results:
            for key in ("pass", "text_equal", "plain_text_equal", "capture_matches_plain", "tensor_sequence_equal"):
                self.assertIs(result[key], True, (result["id"], key))
            self.assertEqual(result["failed_tensors"], 0)
            self.assertEqual(result["exit_codes"], [0, 0])
        for result in read(EVIDENCE / "reference/results.json"):
            self.assertEqual(result["status"], "ok")

    def test_resource_and_protocol_bindings(self):
        for role in ("prepare", "reference", "native"):
            inspect = read(EVIDENCE / (role + "-docker-inspect.json"))[0]
            self.assertEqual(inspect["State"]["ExitCode"], 0)
            self.assertFalse(inspect["State"]["OOMKilled"])
            self.assertEqual(inspect["HostConfig"]["NanoCpus"], 1000000000)
            self.assertEqual(inspect["HostConfig"]["Memory"], 3221225472)
            self.assertEqual(inspect["HostConfig"]["MemorySwap"], 3221225472)
            self.assertEqual(inspect["HostConfig"]["CpusetCpus"], "0")
            self.assertEqual(inspect["HostConfig"]["NetworkMode"], "none")
            self.assertTrue(inspect["HostConfig"]["ReadonlyRootfs"])
        amended = read(EVIDENCE / "amendment.json")
        self.assertEqual(amended["adapter_sha256"], sha(ROOT / "replay.py"))
        self.assertEqual(amended["fresh_environment_sha256"], sha(EVIDENCE / "environment.json"))
        protocol = read(EVIDENCE / "comparison-protocol.json")
        self.assertEqual(protocol["amendment_sha256"], sha(EVIDENCE / "amendment.json"))
        native = read(EVIDENCE / "native/environment.json")
        self.assertEqual(native["protocol_sha256"], sha(EVIDENCE / "comparison-protocol.json"))
        self.assertEqual(native["binary_sha256"], amended["native_binary_sha256"])

    def test_raw_inventory_binds_every_committed_original_receipt(self):
        inventory = read(EVIDENCE / "raw-inventory.json")
        summary = read(EVIDENCE / "summary.json")
        self.assertEqual(summary["raw_inventory_sha256"], sha(EVIDENCE / "raw-inventory.json"))
        self.assertEqual(summary["raw_files"], len(inventory["files"]))
        self.assertEqual(summary["raw_bytes"], sum(r["bytes"] for r in inventory["files"]))
        for record in inventory["files"]:
            path = EVIDENCE / record["path"]
            if path.is_file():
                self.assertEqual(path.stat().st_size, record["bytes"], record["path"])
                self.assertEqual(sha(path), record["sha256"], record["path"])


if __name__ == "__main__":
    unittest.main()
