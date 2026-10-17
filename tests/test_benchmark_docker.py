import tempfile
import unittest
from fractions import Fraction
from pathlib import Path

from benchmarks.docker_run import (admit_cpu_policy, cgroup_files, container_cpu_limits,
                                   enforce_parent_path, validate_cpu_group, validate_leaf_cpu_group)


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

    def test_deployment_cpu_admission_is_exact_and_shared_remains_unpartitioned(self):
        services = {"tika": {"cpus": .25}, "indra": {"cpus": .75}}
        self.assertEqual(admit_cpu_policy(services, 1, "deployment"), {"tika": Fraction(1, 4), "indra": Fraction(3, 4)})
        self.assertEqual(admit_cpu_policy(services, 1, "shared"), {"tika": Fraction(1), "indra": Fraction(1)})
        decimals = {name: {"cpus": value} for name, value in zip(("a", "b", "c"), (.1, .2, .7))}
        self.assertEqual(sum(admit_cpu_policy(decimals, 1, "deployment").values()), 1)
        services["indra"]["cpus"] = "0.74999999999999999999999999"
        with self.assertRaisesRegex(ValueError, "sum exactly"):
            admit_cpu_policy(services, 1, "deployment")
        with self.assertRaisesRegex(ValueError, "sum exactly"):
            admit_cpu_policy({"tika": {"cpus": .5}, "indra": {"cpus": 1.75}}, 2, "deployment")

    def test_deployment_cpu_admission_rejects_missing_unlimited_and_conflicting_limits(self):
        for service in ({}, {"cpus": 0}, {"cpus": -1}, {"cpus": "NaN"}, {"cpus": "Infinity"},
                        {"cpus": 1, "deploy": {"resources": {"limits": {"cpus": ".5"}}}},
                        {"cpus": 1, "cpu_quota": 100000}, {"cpus": 1, "scale": 2},
                        {"cpus": 1, "deploy": {"replicas": 2}}):
            with self.subTest(service=service), self.assertRaises(ValueError):
                admit_cpu_policy({"tika": service}, 1, "deployment")
        matching = {"tika": {"cpus": 1, "deploy": {"resources": {"limits": {"cpus": "1.0"}}}}}
        self.assertEqual(admit_cpu_policy(matching, 1, "deployment"), {"tika": Fraction(1)})

    def test_leaf_limits_must_match_service_identity_and_kernel_quota_exactly(self):
        services = {"tika": Fraction(1, 4), "indra": Fraction(3, 4)}
        containers = [{"Id": "a", "Name": "/project-indra", "Service": "indra"},
                      {"Id": "b", "Name": "/project-tika", "Service": "tika"}]
        limits = container_cpu_limits(containers, services)
        self.assertEqual(limits["/project-tika"], Fraction(1, 4))
        validate_leaf_cpu_group({"cpu.max": "25000 100000", "cpuset.cpus.effective": "2"}, limits["/project-tika"], {2})
        # All these quotas fit the total budget, but violate the admitted partition.
        for quota in ("100000 100000", "75000 100000", "24000 100000", "max 100000", "0 100000"):
            with self.subTest(quota=quota), self.assertRaisesRegex(ValueError, "admitted service"):
                validate_leaf_cpu_group({"cpu.max": quota, "cpuset.cpus.effective": "2"}, limits["/project-tika"], {2})
        containers[1]["Service"] = "indra"
        with self.assertRaisesRegex(ValueError, "admitted one-container"):
            container_cpu_limits(containers, services)


if __name__ == "__main__":
    unittest.main()
