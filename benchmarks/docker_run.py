#!/usr/bin/env python3
"""Verify signed OCI digests, constrain a Compose stack, run a bounded load command.

The supplied Compose file must resolve every service image to an immutable
registry digest. The same CPU set is applied to every service, so their combined
simultaneous execution is bounded by that set's width. An optional parent cgroup
adds aggregate quota accounting. This tool was not executed in the research
environment: its read-only cgroup hierarchy prevents application-container
creation. The default shared policy gives every service the full CPU ceiling.
The deployment policy preserves explicit service quotas whose exact sum equals
the total budget. Both policies retain the common CPU set and parent accounting.
"""

import argparse
import hashlib
import json
import math
import os
import re
import signal
import subprocess
import sys
import threading
import time
from decimal import Decimal, InvalidOperation
from fractions import Fraction
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from benchmarks.procstats import cpu_set


PUBLIC_ENV_KEYS = frozenset({
    "INDRA_WORKERS", "INDRA_THREADS", "INDRA_MATH", "INDRA_RECOGNITION_BATCH", "INDRA_BATCH_METRICS",
    "INDRA_QUEUE_CAPACITY", "INDRA_QUEUE_TIMEOUT_SECONDS", "INDRA_REQUEST_TIMEOUT_SECONDS",
    "INDRA_READ_TIMEOUT_SECONDS", "INDRA_WRITE_TIMEOUT_SECONDS", "INDRA_MAX_REQUEST_BYTES",
    "INDRA_MAX_RESPONSE_BYTES", "INDRA_MAX_PIXELS", "INDRA_MAX_FRAMES",
    "OMP_THREAD_LIMIT", "OMP_NUM_THREADS", "OMP_WAIT_POLICY", "OMP_PROC_BIND", "OMP_PLACES",
    "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS", "NUMEXPR_NUM_THREADS", "VECLIB_MAXIMUM_THREADS",
})


def capture(argv):
    return subprocess.check_output(argv, text=True)


def cgroup_files(path: Path) -> dict:
    result = {}
    for name in ("cpu.max", "cpu.stat", "cpuset.cpus.effective", "memory.current", "memory.peak", "memory.events"):
        try:
            result[name] = (path / name).read_text().strip()
        except OSError as exc:
            if name == "memory.peak":  # Optional on older cgroup-v2 kernels.
                result[name] = None
            else:
                raise RuntimeError(f"required cgroup accounting file unreadable: {path / name}") from exc
    counters = dict(line.split() for line in result["cpu.stat"].splitlines())
    for name in ("usage_usec", "user_usec", "system_usec", "nr_periods", "nr_throttled", "throttled_usec"):
        if name not in counters or int(counters[name]) < 0:
            raise ValueError(f"required CPU accounting counter missing/invalid: {path}/cpu.stat {name}")
    if not result["cpuset.cpus.effective"]:
        raise ValueError(f"empty effective CPU set: {path}")
    return result


def enforce_parent_path(parent: Path, child: Path, root: Path = Path("/sys/fs/cgroup")) -> None:
    parent, child, root = parent.resolve(strict=True), child.resolve(strict=True), root.resolve(strict=True)
    if not parent.is_relative_to(root) or parent not in child.parents:
        raise ValueError("parent-cgroup-path is not an actual ancestor of the container cgroup")


def validate_cpu_group(data: dict, budget: int, cpus: set[int]) -> None:
    quota, period = data["cpu.max"].split()
    if quota == "max" or int(period) <= 0 or int(quota) <= 0 or int(quota) > budget * int(period):
        raise ValueError("cgroup cpu.max does not enforce the requested CPU ceiling")
    if cpu_set(data["cpuset.cpus.effective"]) != cpus:
        raise ValueError("container effective CPU set differs from the requested shared set")


def validate_leaf_cpu_group(data: dict, expected: Fraction, cpus: set[int]) -> None:
    """Check the enforced leaf quota exactly, including an unintended lower cap."""
    quota, period = data["cpu.max"].split()
    if (quota == "max" or int(period) <= 0 or int(quota) <= 0
            or Fraction(int(quota), int(period)) != expected):
        raise ValueError("leaf cpu.max differs from the admitted service CPU quota")
    if cpu_set(data["cpuset.cpus.effective"]) != cpus:
        raise ValueError("container effective CPU set differs from the requested shared set")


def admit_cpu_policy(services: dict, budget: int, policy: str) -> dict[str, Fraction]:
    """Admit one container per service; use rational arithmetic for quota sums."""
    if policy not in ("shared", "deployment") or not services:
        raise ValueError("a known CPU policy and at least one service are required")
    limits = {}
    for name, service in services.items():
        deploy = service.get("deploy", {})
        if service.get("scale", 1) != 1 or deploy.get("replicas", 1) != 1 or deploy.get("mode", "replicated") != "replicated":
            raise ValueError(f"{name}: benchmark admission requires exactly one container per service")
        if policy == "shared":
            limits[name] = Fraction(budget)
            continue
        # cpus is the portable Compose input used by the supplied deployments.
        # Additional quota/period inputs would make preservation ambiguous.
        if any(service.get(field) not in (None, 0) for field in ("cpu_quota", "cpu_period")):
            raise ValueError(f"{name}: deployment admission requires cpus without alternative quota/period inputs")
        try:
            value = Decimal(str(service.get("cpus")))
            if not value.is_finite() or value <= 0:
                raise ValueError("nonpositive or unlimited CPU count")
            limits[name] = Fraction(value)
            other = deploy.get("resources", {}).get("limits", {}).get("cpus")
            if other is not None and Decimal(str(other)) != value:
                raise ValueError("conflicting deploy CPU limit")
        except (InvalidOperation, ValueError) as exc:
            raise ValueError(f"{name}: deployment admission requires one explicit positive, consistent cpus value") from exc
    if policy == "deployment" and sum(limits.values(), Fraction()) != budget:
        raise ValueError("deployment service CPU quotas must sum exactly to the total CPU budget")
    return limits


def container_cpu_limits(containers: list[dict], services: dict[str, Fraction]) -> dict[str, Fraction]:
    """Bind admitted quotas to exact Compose service identities, never list order."""
    if (len(containers) != len(services) or {item.get("Service") for item in containers} != services.keys()
            or len({item["Id"] for item in containers}) != len(services)
            or len({item["Name"] for item in containers}) != len(services)):
        raise ValueError("live containers do not match the admitted one-container-per-service set")
    return {item["Name"]: services[item["Service"]] for item in containers}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compose", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--budget", type=int, choices=(1, 2), required=True)
    parser.add_argument("--cpu-policy", choices=("shared", "deployment"), default="shared")
    parser.add_argument("--cpus", required=True)
    parser.add_argument("--driver-cpu", type=int, required=True)
    parser.add_argument("--identity", required=True, help="exact expected GitHub Actions certificate identity")
    parser.add_argument("--issuer", default="https://token.actions.githubusercontent.com")
    parser.add_argument("--parent-cgroup", help="optional Docker cgroup_parent (driver-specific name)")
    parser.add_argument("--parent-cgroup-path", type=Path, help="host filesystem path for optional aggregate accounting")
    parser.add_argument("--timeout-seconds", type=float, default=1200)
    parser.add_argument("load_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    cpus = cpu_set(args.cpus)
    if len(cpus) != args.budget or args.driver_cpu in cpus:
        parser.error("CPU set width must equal budget and exclude the load generator CPU")
    if not (cpus | {args.driver_cpu}) <= os.sched_getaffinity(0):
        parser.error("requested workload/driver CPUs are unavailable")
    if bool(args.parent_cgroup) != bool(args.parent_cgroup_path):
        parser.error("supply both --parent-cgroup and --parent-cgroup-path for verified aggregate accounting")
    if not math.isfinite(args.timeout_seconds) or args.timeout_seconds <= 0:
        parser.error("timeout must be positive")
    os.sched_setaffinity(0, {args.driver_cpu})
    load = args.load_command[1:] if args.load_command[:1] == ["--"] else args.load_command
    if not load:
        parser.error("supply the bounded load command after --")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    project = f"ocr-benchmark-{os.getpid()}"
    compose = ["docker", "compose", "-p", project, "-f", str(args.compose.resolve())]
    resolved = json.loads(capture(compose + ["config", "--format", "json"]))
    service_limits = admit_cpu_policy(resolved["services"], args.budget, args.cpu_policy)
    (args.output / "cpu-policy.json").write_text(json.dumps({
        "cpu_policy": args.cpu_policy, "total_cpu_budget": args.budget,
        "cpus": args.cpus, "driver_cpu": args.driver_cpu,
        "service_cpu_limits": {name: str(value) for name, value in service_limits.items()},
    }, indent=2) + "\n")
    (args.output / "resolved-compose.sha256").write_text(
        hashlib.sha256(json.dumps(resolved, sort_keys=True, separators=(",", ":")).encode()).hexdigest() + "\n")
    override = {"services": {}}
    images = set()
    for name, service in resolved["services"].items():
        image = service.get("image", "")
        if not re.fullmatch(r"[^\s]+@sha256:[0-9a-f]{64}", image):
            raise ValueError(f"{name}: supply a registry image digest, got {image!r}")
        images.add(image)
        setting = {"cpuset": args.cpus}
        if args.cpu_policy == "shared":
            setting.update(cpus=args.budget, deploy={"resources": {"limits": {"cpus": str(args.budget)}}})
        if args.parent_cgroup:
            setting["cgroup_parent"] = args.parent_cgroup
        override["services"][name] = setting
    for index, image in enumerate(sorted(images)):
        signature = capture(["cosign", "verify", "--certificate-identity", args.identity,
                             "--certificate-oidc-issuer", args.issuer, image])
        (args.output / f"signature-{index}.json").write_text(signature)
    if args.parent_cgroup_path:
        quota, period = (args.parent_cgroup_path / "cpu.max").read_text().split()
        if quota == "max" or int(quota) / int(period) > args.budget:
            raise ValueError("parent cpu.max does not enforce the requested total CPU budget")
    override_path = args.output / "override.json"
    override_path.write_text(json.dumps(override, indent=2) + "\n")
    public_config = json.loads(json.dumps(resolved))
    for service in public_config["services"].values():
        if "environment" in service:
            service["environment"] = {key: value if key in PUBLIC_ENV_KEYS else "<redacted>"
                                      for key, value in service["environment"].items()}
    (args.output / "resolved-compose.json").write_text(json.dumps(public_config, indent=2) + "\n")
    compose += ["-f", str(override_path)]
    monitor_stop = threading.Event()
    monitor = None
    monitor_errors = []
    try:
        subprocess.run(compose + ["up", "-d", "--no-build", "--wait", "--wait-timeout", "180"], check=True)
        ids = capture(compose + ["ps", "-q"]).split()
        containers = json.loads(capture(["docker", "inspect", *ids]))
        public_containers = [{"Id": item["Id"], "Name": item["Name"], "Image": item["Image"],
                              "Service": item.get("Config", {}).get("Labels", {}).get("com.docker.compose.service"),
                              "State": item["State"], "HostConfig": {key: item["HostConfig"].get(key)
                              for key in ("CpusetCpus", "NanoCpus", "CpuQuota", "CpuPeriod", "CgroupParent", "Memory")}}
                             for item in containers]
        (args.output / "containers.json").write_text(json.dumps(public_containers, indent=2) + "\n")
        leaf_limits = container_cpu_limits(public_containers, service_limits)
        paths = {}
        for container in containers:
            if cpu_set(container["HostConfig"]["CpusetCpus"]) != cpus:
                raise ValueError("container CPU affinity configuration differs from requested shared set")
            pid = container["State"]["Pid"]
            rows = Path(f"/proc/{pid}/cgroup").read_text().splitlines()
            unified = [row.split(":", 2)[2] for row in rows if row.startswith("0::")]
            if len(unified) != 1:
                raise ValueError("benchmark accounting requires cgroup v2 on the Docker host")
            path = (Path("/sys/fs/cgroup") / unified[0].lstrip("/")).resolve(strict=True)
            validate_leaf_cpu_group(cgroup_files(path), leaf_limits[container["Name"]], cpus)
            if args.parent_cgroup_path:
                enforce_parent_path(args.parent_cgroup_path, path)
            paths[container["Name"]] = path
        if args.parent_cgroup_path:
            paths["aggregate_parent"] = args.parent_cgroup_path
            cgroup_files(args.parent_cgroup_path)  # Readability is a gate, not a best-effort sample.

        def sample():
            with (args.output / "cgroups.jsonl").open("w") as stream:
                while not monitor_stop.is_set():
                    try:
                        groups = {name: cgroup_files(path) for name, path in paths.items()}
                        for name, data in groups.items():
                            if name == "aggregate_parent":
                                validate_cpu_group(data, args.budget, cpus)
                            else:
                                validate_leaf_cpu_group(data, leaf_limits[name], cpus)
                        stream.write(json.dumps({"monotonic_seconds": time.monotonic(), "groups": groups}) + "\n")
                        stream.flush()
                    except (OSError, ValueError, RuntimeError) as exc:
                        monitor_errors.append(f"{type(exc).__name__}: {exc}")
                        monitor_stop.set()
                        break
                    monitor_stop.wait(.25)

        monitor = threading.Thread(target=sample, daemon=True)
        monitor.start()
        with (args.output / "load.stdout").open("wb") as stdout, (args.output / "load.stderr").open("wb") as stderr:
            proc = subprocess.Popen(["taskset", "--cpu-list", str(args.driver_cpu), *load],
                                    stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                deadline = time.monotonic() + args.timeout_seconds
                while proc.poll() is None:
                    if monitor_errors:
                        raise RuntimeError("cgroup accounting failed: " + monitor_errors[0])
                    if time.monotonic() >= deadline:
                        return 124
                    monitor_stop.wait(.05)
                monitor_stop.set()
                monitor.join(timeout=2)
                if monitor_errors:
                    raise RuntimeError("cgroup accounting failed: " + monitor_errors[0])
                return proc.returncode
            finally:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait(timeout=5)
    finally:
        monitor_stop.set()
        if monitor:
            monitor.join(timeout=2)
        if monitor_errors:
            (args.output / "accounting-error.json").write_text(json.dumps(monitor_errors) + "\n")
        logs = subprocess.run(compose + ["logs", "--no-color"], capture_output=True, text=True)
        (args.output / "services.log").write_text(logs.stdout + logs.stderr)
        subprocess.run(compose + ["down", "--remove-orphans"], check=False)


if __name__ == "__main__":
    raise SystemExit(main())
