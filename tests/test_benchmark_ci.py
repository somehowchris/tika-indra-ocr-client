"""Critical arithmetic and evidence guards for the signed-container CI gate."""

import argparse
import copy
import http.client
import json
from fractions import Fraction
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch
import xml.etree.ElementTree as ET

from benchmarks.ci_docker import check_summary, image_reference, parent_data, resource_delta, snapshot, wait_tika


ROOT = Path(__file__).resolve().parents[1]


def group(usage=0, oom=0):
    return {"cpu.stat": f"usage_usec {usage}\nuser_usec {usage}\nsystem_usec 0\nnr_periods 1\nnr_throttled 0\nthrottled_usec 0",
            "memory.events": f"low 0\nhigh 0\nmax 0\noom {oom}\noom_kill 0\noom_group_kill 0"}


class ContainerBenchmarkGuards(unittest.TestCase):
    def test_aggregate_parent_is_not_double_counted_with_children(self):
        before = {"read_started_monotonic": 1, "groups": {"tika": group(100), "indra": group(200), "aggregate_parent": group(320)}}
        after = {"read_finished_monotonic": 2, "groups": {"tika": group(300), "indra": group(500), "aggregate_parent": group(840)}}
        result = resource_delta(before, after)
        self.assertEqual(result["cpu_seconds"], .000520)
        self.assertEqual(result["summed_container_cpu_counters"]["usage_usec"], 500)
        self.assertNotIn("aggregate_parent", result["per_container_cpu_counters"])

    def test_counter_resets_and_oom_fail_the_gate(self):
        before = {"read_started_monotonic": 1, "groups": {"aggregate_parent": group(10)}}
        with self.assertRaisesRegex(ValueError, "regressed"):
            resource_delta(before, {"read_finished_monotonic": 2, "groups": {"aggregate_parent": group(5)}})
        with self.assertRaisesRegex(ValueError, "OOM"):
            resource_delta(before, {"read_finished_monotonic": 2, "groups": {"aggregate_parent": group(20, oom=1)}})

    def test_exact_parent_cpu_memory_and_swap_limits(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            values = group() | {"cpu.max": "200000 100000", "cpuset.cpus.effective": "2-3",
                               "memory.current": "0", "memory.peak": "0", "memory.max": "3221225472", "memory.swap.max": "0"}
            for name, value in values.items():
                (path / name).write_text(value)
            self.assertEqual(parent_data(path, 2, {2, 3})["memory.max"], "3221225472")
            (path / "cpu.max").write_text("100000 100000")
            with self.assertRaisesRegex(ValueError, "exact requested budget"):
                parent_data(path, 2, {2, 3})
            (path / "cpu.max").write_text("200000 100000")
            (path / "memory.max").write_text("max")
            with self.assertRaisesRegex(ValueError, "memory.max"):
                parent_data(path, 2, {2, 3})
            (path / "memory.max").write_text("3221225472")
            (path / "memory.swap.max").write_text("1024")
            with self.assertRaisesRegex(ValueError, "memory.swap.max"):
                parent_data(path, 2, {2, 3})

    def test_mutable_tags_and_option_strings_are_rejected(self):
        valid = "ghcr.io/example/ocr@sha256:" + "a" * 64
        self.assertEqual(image_reference(valid), valid)
        for invalid in ("ghcr.io/example/ocr:latest", "--flag@sha256:" + "a" * 64):
            with self.assertRaises(argparse.ArgumentTypeError):
                image_reference(invalid)

    def test_readiness_retries_startup_disconnects_but_rejects_wrong_version(self):
        response = MagicMock()
        response.__enter__.return_value = response
        response.status = 200
        response.read.return_value = b"Apache Tika 3.3.2"
        errors = [ConnectionResetError("starting"), http.client.RemoteDisconnected("starting")]
        with patch("benchmarks.ci_docker.urllib.request.urlopen", side_effect=[*errors, response]), \
                patch("benchmarks.ci_docker.time.sleep") as sleep:
            self.assertGreaterEqual(wait_tika("http://127.0.0.1:9998/tika", float("inf")), 0)
            self.assertEqual(sleep.call_count, 2)
        response.read.return_value = b"Apache Tika 2.9.2"
        with patch("benchmarks.ci_docker.urllib.request.urlopen", return_value=response):
            with self.assertRaisesRegex(ValueError, "expected Tika 3.3.2"):
                wait_tika("http://127.0.0.1:9998/tika", float("inf"))

    def test_measurement_snapshot_rejects_leaf_partition_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            parent, leaf = Path(directory), Path(directory) / "tika"
            leaf.mkdir()
            values = group() | {"cpu.max": "100000 100000", "cpuset.cpus.effective": "2",
                               "memory.current": "0", "memory.peak": "0", "memory.max": "3221225472", "memory.swap.max": "0"}
            for path in (parent, leaf):
                for name, value in values.items():
                    (path / name).write_text(value)
            (leaf / "cpu.max").write_text("25000 100000")
            paths, limits = {"aggregate_parent": parent, "tika": leaf}, {"tika": Fraction(1, 4)}
            snapshot(paths, 1, {2}, limits)
            (leaf / "cpu.max").write_text("50000 100000")
            with self.assertRaisesRegex(ValueError, "admitted service"):
                snapshot(paths, 1, {2}, limits)

    def test_archived_accuracy_is_within_explicit_smoke_guards_and_blank_errors_fail(self):
        summary = json.loads((ROOT / "benchmarks/results/tika-1/summary.json").read_text())
        self.assertEqual(check_summary(summary, 3), [])
        bad = copy.deepcopy(summary)
        bad["engines"]["tika-indra"]["accuracy"]["blank_false_positives"] = 1
        self.assertTrue(any("blank page" in error for error in check_summary(bad, 3)))
        bad = copy.deepcopy(summary)
        bad["engines"]["tika-indra"]["attempts"] -= 1
        self.assertTrue(any("incomplete" in error for error in check_summary(bad, 3)))

    def test_compared_tika_server_and_pdf_settings_match(self):
        candidate = ET.parse(ROOT / "deploy/tika-benchmark.xml").getroot()
        baseline = ET.parse(ROOT / "deploy/tika-tesseract.xml").getroot()
        self.assertEqual(ET.tostring(candidate.find("server")), ET.tostring(baseline.find("server")))
        selector = "parsers/parser[@class='org.apache.tika.parser.pdf.PDFParser']"
        self.assertEqual(ET.tostring(candidate.find(selector)), ET.tostring(baseline.find(selector)))


if __name__ == "__main__":
    unittest.main()
