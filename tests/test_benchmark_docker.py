import tempfile
import unittest
from pathlib import Path

from benchmarks.docker_run import cgroup_files, enforce_parent_path, validate_cpu_group


class DockerAccountingTests(unittest.TestCase):
    def test_missing_cpu_accounting_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "required cgroup accounting file unreadable"):
                cgroup_files(Path(directory))

    def test_parent_must_be_actual_ancestor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            actual, unrelated = root / "actual", root / "unrelated"
            child = actual / "container"
            child.mkdir(parents=True)
            unrelated.mkdir()
            enforce_parent_path(actual, child, root)
            with self.assertRaisesRegex(ValueError, "actual ancestor"):
                enforce_parent_path(unrelated, child, root)

    def test_effective_mask_and_quota_must_match_budget(self):
        validate_cpu_group({"cpu.max": "200000 100000", "cpuset.cpus.effective": "2-3"}, 2, {2, 3})
        for data in ({"cpu.max": "max 100000", "cpuset.cpus.effective": "2"},
                     {"cpu.max": "200000 100000", "cpuset.cpus.effective": "2"},
                     {"cpu.max": "100000 100000", "cpuset.cpus.effective": "2-3"}):
            with self.assertRaises(ValueError):
                validate_cpu_group(data, 1, {2})


if __name__ == "__main__":
    unittest.main()
