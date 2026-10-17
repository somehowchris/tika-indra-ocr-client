import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from benchmarks.metrics import score
from benchmarks.procstats import cpu_set
from benchmarks.run import load_cases, summarize


class RunnerTests(unittest.TestCase):
    def test_cpu_list(self):
        self.assertEqual(cpu_set("2,4-5"), {2, 4, 5})
        with self.assertRaises(ValueError):
            cpu_set("3-2")

    def test_manifest_hash_mismatch_rejected_before_engine(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "image.bin").write_bytes(b"fixture")
            (root / "truth.txt").write_text("truth")
            path = root / "manifest.json"
            path.write_text(json.dumps({"schema_version": 1, "cases": [
                {"id": "one", "input": "image.bin", "truth": "truth.txt", "input_sha256": "wrong"}]}))
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                load_cases(path, 5, 1024)

    def test_repetitions_do_not_inflate_accuracy_and_failures_are_visible(self):
        rows = []
        for repetition in range(3):
            rows.append({"engine": "fixture-engine", "case_id": "a", "document_id": "doc-a",
                         "kind": "synthetic", "pages": 1, "repetition": repetition, "phase": "measured",
                         "status": "error" if repetition == 0 else "ok", "latency_ms": 10,
                         "output_sha256": "same", "score": score("truth", "" if repetition == 0 else "truth"),
                         "strict_score": score("truth", "" if repetition == 0 else "truth")})
        args = SimpleNamespace(cpus="0", whitespace="collapse", seed=17)
        result = summarize(rows, [{"engine": "fixture-engine", "measured_wall_seconds": 1}], ["fixture-engine"], args)
        summary = result["engines"]["fixture-engine"]
        self.assertEqual(summary["attempts"], 3)
        self.assertEqual(summary["failures"], 1)
        self.assertEqual(summary["accuracy"]["unique_cases"], 1)
        self.assertEqual(summary["accuracy"]["micro_cer"], 1)

    @unittest.skipUnless(hasattr(os, "sched_getaffinity"), "Linux affinity test")
    def test_runner_propagates_command_failure_and_retains_raw_output(self):
        cpus = sorted(os.sched_getaffinity(0))
        if len(cpus) < 2:
            self.skipTest("separate workload/driver CPUs unavailable")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "input.bin").write_bytes(b"not an image: command contract test only")
            (root / "truth.txt").write_text("expected text", encoding="utf-8")
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"schema_version": 1, "synthetic": True, "cases": [
                {"id": "case", "input": "input.bin", "truth": "truth.txt"}]}))
            plan = root / "plan.json"
            plan.write_text(json.dumps({"engines": [{"name": "intentional-error-fixture", "mode": "command",
                "command": [sys.executable, "-c", "print('partial text'); raise SystemExit(7)"]}]}))
            output = root / "result"
            proc = subprocess.run([sys.executable, "-m", "benchmarks.run", "--manifest", str(manifest),
                "--plan", str(plan), "--output", str(output), "--cpus", str(cpus[0]), "--driver-cpu", str(cpus[-1]),
                "--warmup", "0", "--repeats", "1", "--timeout", "5", "--max-run-seconds", "10"],
                capture_output=True, text=True, timeout=20)
            self.assertEqual(proc.returncode, 1, proc.stderr)
            summary = json.loads((output / "summary.json").read_text())["engines"]["intentional-error-fixture"]
            self.assertEqual(summary["failures"], 1)
            self.assertEqual(summary["accuracy"]["micro_cer"], 1)
            self.assertIn("partial text", (output / "intentional-error-fixture-0/measured-case.txt").read_text())


if __name__ == "__main__":
    unittest.main()
