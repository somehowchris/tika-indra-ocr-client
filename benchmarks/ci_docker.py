#!/usr/bin/env python3
"""Measure signed Docker candidates with one shared 1/2-CPU application set.

The matrix uses the committed, hashed 15-case synthetic corpus, three fresh
container cohorts per engine/budget, and the existing Docker ownership,
signature, resource-accounting, HTTP, and scoring code. It deliberately does
not accept unsigned development tags. The release workflow invokes this gate
after signature verification. The load subcommand is internal: it only
operates on containers already owned and verified by docker_run.py.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import contextlib
import json
import os
import random
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
if __package__ in (None, ""):
    sys.path.insert(0, str(ROOT))

from benchmarks.docker_run import (admit_cpu_policy, cgroup_files, container_cpu_limits,
                                   enforce_parent_path, validate_cpu_group, validate_leaf_cpu_group)
from benchmarks.procstats import cpu_set
from benchmarks.run import digest, host_metadata, load_cases, read_json, run_case, save_json, score_result, summarize

ENGINES = ("tika-tesseract-best", "tika-indra")
CORPUS = ROOT / "benchmarks/results/synthetic-fixtures/manifest.json"
ENFORCEMENT = (
    "Docker cgroup v2: all application containers descend from one verified "
    "systemd slice with aggregate cpu.max=budget*100000/100000, "
    "memory.max=3221225472, and memory.swap.max=0. The parent and every "
    "container share a cpuset of width equal to the total CPU budget. "
    "Driver and monitor use a disjoint CPU outside that parent."
)
CPU_POLICIES = {
    "shared": "Each container receives the full CPU ceiling; the aggregate parent and common CPU set enforce the total budget. This overrides the supplied deployment CPU partitions.",
    "deployment": "The supplied per-service CPU quotas are preserved, sum exactly to the total budget, and are verified against each kernel leaf cpu.max. The aggregate parent and common CPU set also enforce the total budget.",
}


def capture(argv: list[str]) -> str:
    return subprocess.check_output(argv, text=True, timeout=180, cwd=ROOT)


def image_reference(value: str) -> str:
    if not re.fullmatch(r"[a-z0-9][^\s]*@sha256:[0-9a-f]{64}", value):
        raise argparse.ArgumentTypeError("an immutable registry image@sha256:digest is required")
    return value


def parent_data(path: Path, budget: int, allowed: set[int]) -> dict:
    data = cgroup_files(path)
    validate_cpu_group(data, budget, allowed)
    if tuple(map(int, data["cpu.max"].split())) != (budget * 100000, 100000):
        raise ValueError("parent CPU quota/period differs from the exact requested budget")
    for filename, expected in (("memory.max", 3 * 1024 ** 3), ("memory.swap.max", 0)):
        value = (path / filename).read_text().strip()
        if value != str(expected):
            raise ValueError(f"parent {filename} differs from {expected}")
        data[filename] = value
    events = counters(data["memory.events"])
    if any(events.get(name, 0) for name in ("oom", "oom_kill", "oom_group_kill")):
        raise ValueError("aggregate application parent recorded an OOM event")
    return data


@contextlib.contextmanager
def shared_parent(directory: Path, budget: int, cpus: str):
    """Own a runtime-only systemd slice; never change existing unit files."""
    name = f"ocrbenchmark{uuid.uuid4().hex}.slice"
    source = directory / name
    runtime = Path("/run/systemd/system") / name
    source.write_text(
        "[Unit]\nDescription=Bounded OCR candidate benchmark\n\n[Slice]\n"
        f"CPUAccounting=yes\nCPUQuota={budget * 100}%\nCPUQuotaPeriodSec=100ms\nAllowedCPUs={cpus}\n"
        "MemoryAccounting=yes\nMemoryMax=3221225472\nMemorySwapMax=0\n"
    )
    privilege = [] if os.geteuid() == 0 else ["sudo", "-n"]
    installed = False
    started = False
    try:
        subprocess.run(privilege + ["install", "--mode=0644", str(source), str(runtime)], check=True, timeout=30)
        installed = True
        subprocess.run(privilege + ["systemctl", "daemon-reload"], check=True, timeout=30)
        subprocess.run(privilege + ["systemctl", "start", name], check=True, timeout=30)
        started = True
        control_group = capture(["systemctl", "show", "--property=ControlGroup", "--value", name]).strip()
        if control_group != "/" + name:  # The generated name has no hierarchy-encoding dashes.
            raise ValueError("systemd returned an unexpected parent cgroup path")
        path = (Path("/sys/fs/cgroup") / name).resolve(strict=True)
        save_json(directory / "parent-before.json", {"unit": name, "path": str(path), **parent_data(path, budget, cpu_set(cpus))})
        yield name, path
        final = parent_data(path, budget, cpu_set(cpus))
        final["cgroup.events"] = (path / "cgroup.events").read_text().strip()
        save_json(directory / "parent-after.json", final)
        if counters(final["cgroup.events"])["populated"]:
            raise ValueError("application processes remained in the parent after stack cleanup")
    finally:
        cleanup = []
        if started:
            result = subprocess.run(privilege + ["systemctl", "stop", name], capture_output=True, text=True, timeout=30)
            cleanup.append({"operation": "stop owned slice", "returncode": result.returncode, "stderr": result.stderr})
        if installed:
            for operation, argv in (("remove owned runtime unit", ["rm", "--", str(runtime)]),
                                    ("reload runtime units", ["systemctl", "daemon-reload"])):
                result = subprocess.run(privilege + argv, capture_output=True, text=True, timeout=30)
                cleanup.append({"operation": operation, "returncode": result.returncode, "stderr": result.stderr})
        save_json(directory / "parent-cleanup.json", cleanup)
        if any(item["returncode"] for item in cleanup) and sys.exc_info()[0] is None:
            raise RuntimeError("systemd parent cleanup failed; see parent-cleanup.json")


def run_wrapper(argv: list[str], compose: Path, directory: Path) -> int:
    """Bound the complete wrapper and clean its exact project on cancellation."""
    with (directory / "wrapper.log").open("w") as log:
        process = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT, cwd=ROOT, start_new_session=True)
        status = 124
        try:
            status = process.wait(timeout=1100)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
            # docker_run.py derives this project from its PID; taskset execs it.
            cleanup = subprocess.run(["docker", "compose", "-p", f"ocr-benchmark-{process.pid}",
                                      "-f", str(compose), "down", "--remove-orphans", "--timeout", "10"],
                                     stdout=log, stderr=subprocess.STDOUT, timeout=90, cwd=ROOT)
            if cleanup.returncode:
                status = cleanup.returncode
        return status


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def prepared_compose(engine: str, budget: int, port: int, refs: dict) -> dict:
    files = ["compose.yaml", "compose.benchmark.yaml"] if engine == "tika-indra" else ["compose.tesseract.yaml"]
    if budget == 2:
        files.append("compose.2cpu.yaml" if engine == "tika-indra" else "compose.tesseract.2cpu.yaml")
    argv = ["docker", "compose"]
    for path in files:
        argv.extend(("-f", str(ROOT / path)))
    result = json.loads(capture(argv + ["config", "--format", "json"]))
    result.pop("name", None)
    for network in result.get("networks", {}).values():
        if not network.get("external"):
            network.pop("name", None)  # Let the wrapper's unique project own its network.
    for name, service in result["services"].items():
        service.pop("build", None)
        service["restart"] = "no"  # A crash must not disappear behind automatic restart.
        service["image"] = refs["backend"] if name == "indra" else refs["tika" if engine == "tika-indra" else "baseline"]
    result["services"]["tika"]["ports"] = [{"target": 9998, "published": str(port), "host_ip": "127.0.0.1", "protocol": "tcp"}]
    if engine == "tika-indra":
        result["services"]["indra"]["environment"].update({
            "INDRA_WORKERS": str(budget), "INDRA_THREADS": "1", "INDRA_MATH": "0",
            "INDRA_RECOGNITION_BATCH": "64", "INDRA_BATCH_METRICS": "1",
        })
    return result


def owned_groups(directory: Path, parent: Path, budget: int, allowed: set[int], expected_count: int,
                 policy: str) -> tuple[dict, dict]:
    containers = read_json(directory / "containers.json")
    if len(containers) != expected_count or len({c["Id"] for c in containers}) != expected_count:
        raise ValueError("unexpected application-container count")
    service_limits = admit_cpu_policy(read_json(directory / "resolved-compose.json")["services"], budget, policy)
    recorded = read_json(directory / "cpu-policy.json")
    if (recorded["cpu_policy"] != policy or recorded["total_cpu_budget"] != budget
            or cpu_set(recorded["cpus"]) != allowed
            or recorded["service_cpu_limits"] != {name: str(value) for name, value in service_limits.items()}):
        raise ValueError("wrapper CPU-policy evidence differs from the requested cohort")
    leaf_limits = container_cpu_limits(containers, service_limits)
    paths = {}
    for container in containers:
        if not container["State"]["Running"] or container["State"].get("OOMKilled"):
            raise ValueError("application container did not start cleanly")
        pid = container["State"]["Pid"]
        rows = Path(f"/proc/{pid}/cgroup").read_text().splitlines()
        unified = [row.split(":", 2)[2] for row in rows if row.startswith("0::")]
        if len(unified) != 1:
            raise ValueError("host-visible cgroup v2 is required")
        path = (Path("/sys/fs/cgroup") / unified[0].lstrip("/")).resolve(strict=True)
        validate_leaf_cpu_group(cgroup_files(path), leaf_limits[container["Name"]], allowed)
        enforce_parent_path(parent, path)
        paths[container["Name"]] = path
    values = list(paths.values())
    if len(set(values)) != expected_count or any(a in b.parents for a in values for b in values if a != b):
        raise ValueError("application cgroups must be distinct and non-overlapping for additive accounting")
    driver_rows = Path("/proc/self/cgroup").read_text().splitlines()
    driver_unified = [row.split(":", 2)[2] for row in driver_rows if row.startswith("0::")]
    if len(driver_unified) != 1:
        raise ValueError("cannot determine the driver cgroup")
    driver_path = (Path("/sys/fs/cgroup") / driver_unified[0].lstrip("/")).resolve(strict=True)
    if driver_path.is_relative_to(parent):
        raise ValueError("load generator must be outside the aggregate application parent")
    parent_data(parent, budget, allowed)
    paths["aggregate_parent"] = parent
    return paths, leaf_limits


def snapshot(paths: dict[str, Path], budget: int, allowed: set[int], leaf_limits: dict) -> dict:
    started = time.monotonic()
    groups = {name: parent_data(path, budget, allowed) if name == "aggregate_parent" else cgroup_files(path)
              for name, path in paths.items()}
    for name, data in groups.items():
        if name != "aggregate_parent":
            validate_leaf_cpu_group(data, leaf_limits[name], allowed)
    return {"read_started_monotonic": started, "read_finished_monotonic": time.monotonic(), "groups": groups}


def counters(value: str) -> dict[str, int]:
    return {name: int(amount) for name, amount in (line.split() for line in value.splitlines())}


def resource_delta(before: dict, after: dict) -> dict:
    if before["groups"].keys() != after["groups"].keys():
        raise ValueError("application cgroup set changed during measurement")
    totals = {name: 0 for name in ("usage_usec", "user_usec", "system_usec", "nr_periods", "nr_throttled", "throttled_usec")}
    per_container = {}
    for group, first in before["groups"].items():
        last = after["groups"][group]
        a, b = counters(first["cpu.stat"]), counters(last["cpu.stat"])
        delta = {name: b[name] - a[name] for name in totals}
        if any(value < 0 for value in delta.values()):
            raise ValueError("cgroup CPU accounting regressed/reset")
        events = counters(last["memory.events"])
        if any(events.get(name, 0) for name in ("oom", "oom_kill", "oom_group_kill")):
            raise ValueError("application cgroup recorded an OOM event")
        per_container[group] = delta
        if group != "aggregate_parent":
            for name, value in delta.items():
                totals[name] += value
    parent = per_container.pop("aggregate_parent")
    if parent["usage_usec"] <= 0:
        raise ValueError("no aggregate CPU consumption was accounted during OCR")
    return {"summed_container_cpu_counters": totals, "per_container_cpu_counters": per_container,
            "aggregate_parent_cpu_counters": parent, "cpu_seconds": parent["usage_usec"] / 1_000_000,
            "counter_interval_wall_seconds": after["read_finished_monotonic"] - before["read_started_monotonic"],
            "scope": "sequential snapshots immediately around the measured HTTP cohort; includes background application work"}


def wait_tika(url: str, deadline: float) -> float:
    started = time.monotonic()
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url.rsplit("/", 1)[0] + "/version", timeout=1) as response:
                version = response.read(4096).decode("utf-8")
                if response.status != 200 or "3.3.2" not in version:
                    raise ValueError(f"expected Tika 3.3.2, received {version!r}")
                return (time.monotonic() - started) * 1000
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            time.sleep(.1)
    raise TimeoutError("Tika did not become ready within the bounded readiness window")


def load_cohort(args) -> int:
    if os.sched_getaffinity(0) != {args.driver_cpu}:
        raise ValueError("Docker wrapper must pin the load generator to its disjoint driver CPU")
    allowed = cpu_set(args.cpus)
    if len(allowed) != args.budget or args.driver_cpu in allowed:
        raise ValueError("invalid application/driver CPU assignment")
    args.output.mkdir(parents=True, exist_ok=False)
    _, cases = load_cases(CORPUS, 16, 64 * 1024 * 1024)
    if len(cases) != 15:
        raise ValueError("the fixed CI corpus must contain exactly 15 hashed cases")
    paths, leaf_limits = owned_groups(args.wrapper_output, args.parent_cgroup_path, args.budget, allowed,
                                     2 if args.engine == "tika-indra" else 1, args.cpu_policy)
    save_json(args.output / "cgroup-paths.json", {name: str(path) for name, path in paths.items()})
    args.timeout, args.max_output_bytes, args.whitespace = 120, 8 * 1024 * 1024, "collapse"
    deadline = time.monotonic() + 600
    random.Random(args.seed + args.repetition).shuffle(cases)
    engine = {"name": args.engine, "mode": "http", "url": args.url, "method": "PUT"}
    cohort = {"engine": args.engine, "repetition": args.repetition, "concurrency": args.budget,
              "cpu_policy": args.cpu_policy, "leaf_cpu_limits": {name: str(value) for name, value in leaf_limits.items()},
              "selected_cases": [case["id"] for case in cases], "startup_ms": None,
              "startup_note": "Docker startup is outside this client; readiness_wait_ms is only the residual client wait",
              "readiness_wait_ms": wait_tika(args.url, min(deadline, time.monotonic() + 120))}
    records = []
    with (args.output / "requests.jsonl").open("w", encoding="utf-8") as stream:
        warmup = run_case(engine, cases[0], args, args.output, None, deadline, args.repetition, "warmup0")
        cohort["first_request_ms"] = warmup["latency_ms"]
        score_result(warmup, cases[0], args)
        records.append(warmup)
        stream.write(json.dumps(warmup, ensure_ascii=False) + "\n")
        stream.flush()
        before = snapshot(paths, args.budget, allowed, leaf_limits)
        started = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.budget) as pool:
            futures = [pool.submit(run_case, engine, case, args, args.output, None, deadline, args.repetition, "measured") for case in cases]
            measured = [future.result() for future in concurrent.futures.as_completed(futures)]
        finished = time.monotonic()
        after = snapshot(paths, args.budget, allowed, leaf_limits)
        cohort.update(measured_wall_seconds=finished - started, measured_started_monotonic=started,
                      measured_finished_monotonic=finished, measured_resources=resource_delta(before, after),
                      before_measurement=before, after_measurement=after)
        by_id = {case["id"]: case for case in cases}
        for row in measured:
            score_result(row, by_id[row["case_id"]], args)
            records.append(row)
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    save_json(args.output / "cohort.json", cohort)
    return int(any(row["status"] != "ok" for row in records))


def sampled_memory(wrapper: Path, cohort: dict) -> dict:
    rows = [json.loads(line) for line in (wrapper / "cgroups.jsonl").read_text().splitlines()]
    measured = [row for row in rows if cohort["measured_started_monotonic"] <= row["monotonic_seconds"] <= cohort["measured_finished_monotonic"]]
    if not rows or not measured:
        raise ValueError("cgroup monitor produced no samples during the measured cohort")
    return {"samples": len(measured), "sample_interval_seconds": .25,
            "peak_aggregate_memory_current_bytes": max(int(row["groups"]["aggregate_parent"]["memory.current"]) for row in measured),
            "scope": "sampled aggregate-parent memory.current including all application descendants; charged memory, not RSS"}


def check_summary(summary: dict, repeats: int) -> list[str]:
    errors = []
    for name, item in summary["engines"].items():
        if item["attempts"] != 15 * repeats or item["failures"]:
            errors.append(f"{name}: incomplete/failed timed requests")
        if item["nondeterministic_output_case_ids"]:
            errors.append(f"{name}: repeated OCR outputs changed")
        maximum = .01 if name == "tika-indra" else .05
        if item["accuracy"]["micro_cer"] is None or item["accuracy"]["micro_cer"] > maximum:
            errors.append(f"{name}: synthetic CER exceeds the explicit {maximum:.0%} smoke ceiling")
        if item["accuracy"]["blank_false_positives"]:
            errors.append(f"{name}: blank page produced text")
    return errors


def matrix(args) -> int:
    available = sorted(os.sched_getaffinity(0))
    if len(available) < 3:
        raise ValueError("need at least 3 eligible logical CPUs: two application CPUs plus a disjoint driver")
    info = json.loads(capture(["docker", "info", "--format", "{{json .}}"]))
    if str(info.get("CgroupVersion")) != "2":
        raise ValueError("a local Docker host with readable cgroup v2 accounting is required")
    if info.get("CgroupDriver") != "systemd":
        raise ValueError("the Docker daemon must use native.cgroupdriver=systemd for an owned aggregate parent")
    if not Path("/run/systemd/system").is_dir():
        raise ValueError("the runner must expose its native systemd runtime hierarchy")
    if os.geteuid() != 0:
        subprocess.run(["sudo", "-n", "true"], check=True, timeout=10)
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    args.owned_output = True
    shutil.copyfile(ROOT / "licenses/INDRA-MODEL-LICENSE.txt", args.output / "INDRA-MODEL-LICENSE.txt")
    initial_host = host_metadata()
    driver, workload = available[-1], available[:2]
    os.sched_setaffinity(0, {driver})
    refs = {"tika": args.tika_ref, "backend": args.backend_ref, "baseline": args.baseline_ref}
    _, cases = load_cases(CORPUS, 16, 64 * 1024 * 1024)
    if len(cases) != 15:
        raise ValueError("the fixed CI corpus must contain exactly 15 hashed cases")
    sources = ["benchmarks/ci_docker.py", "benchmarks/docker_run.py", "benchmarks/run.py", "benchmarks/metrics.py",
               "compose.yaml", "compose.2cpu.yaml", "compose.benchmark.yaml", "compose.tesseract.yaml",
               "compose.tesseract.2cpu.yaml", "deploy/tika-benchmark.xml", "deploy/tika-tesseract.xml",
               "docker/tika.Dockerfile", "docker/tesseract.Dockerfile", "backend/Dockerfile", "backend/models.lock.json"]
    save_json(args.output / "environment.json", {"host": initial_host, "image_refs": refs,
              "certificate_identity": args.identity, "driver_cpu": driver, "application_cpus": workload,
              "manifest_sha256": digest(CORPUS), "cases": [{k: v for k, v in case.items() if k != "reference"} for case in cases],
              "source_sha256": {name: digest(ROOT / name) for name in sources},
              "model_manifest": read_json(ROOT / "backend/models.lock.json"),
              "github": {name: os.environ.get(name) for name in ("GITHUB_REPOSITORY", "GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT")},
              "enforcement": ENFORCEMENT, "cpu_policy": args.cpu_policy, "cpu_policy_description": CPU_POLICIES[args.cpu_policy],
              "repeats": args.repeats, "seed": args.seed,
              "profiles": "final-image Java settings retained; OCR-only 300 DPI; OMP1; Indra workers=budget, intra=1, batch ceiling64, math off",
              "workload": "closed-loop finite synthetic corpus; accuracy counts repetition0 only; no production/SLO guarantee"})
    summaries, failures = {}, []
    for budget in (1, 2):
        directory = args.output / f"cpu-{budget}"
        directory.mkdir()
        cpus = ",".join(map(str, workload[:budget]))
        records, cohorts = [], []
        base_order = list(ENGINES)
        random.Random(args.seed).shuffle(base_order)
        for repetition in range(args.repeats):
            order = base_order[repetition % 2:] + base_order[:repetition % 2]
            for engine in order:
                cohort_dir = directory / f"{engine}-{repetition}"
                cohort_dir.mkdir()
                port = free_port()
                compose = cohort_dir / "compose.json"
                save_json(compose, prepared_compose(engine, budget, port, refs))
                wrapper = cohort_dir / "docker"
                load = cohort_dir / "load"
                argv = ["taskset", "--cpu-list", ",".join(map(str, available)), sys.executable, str(ROOT / "benchmarks/docker_run.py"),
                        "--compose", str(compose), "--output", str(wrapper), "--budget", str(budget), "--cpu-policy", args.cpu_policy,
                        "--cpus", cpus, "--driver-cpu", str(driver), "--identity", args.identity, "--timeout-seconds", "720", "--",
                        sys.executable, str(Path(__file__).resolve()), "load", "--wrapper-output", str(wrapper), "--output", str(load),
                        "--engine", engine, "--budget", str(budget), "--cpu-policy", args.cpu_policy, "--cpus", cpus, "--driver-cpu", str(driver),
                        "--repetition", str(repetition), "--seed", str(args.seed), "--url", f"http://127.0.0.1:{port}/tika"]
                with shared_parent(cohort_dir, budget, cpus) as (parent_name, parent_path):
                    separator = argv.index("--")
                    argv[separator:separator] = ["--parent-cgroup", parent_name, "--parent-cgroup-path", str(parent_path)]
                    argv.extend(("--parent-cgroup-path", str(parent_path)))
                    status = run_wrapper(argv, compose, cohort_dir)
                    if status:
                        failures.append(f"cpu-{budget}/{engine}/{repetition}: Docker/load gate exited {status}")
                        save_json(args.output / "gate.json", {"status": "failed", "errors": failures, "cpu_policy": args.cpu_policy})
                        return 1  # Keep the failed attempt and stop; no hidden successful retry.
                cohort = read_json(load / "cohort.json")
                cohort["measured_memory"] = sampled_memory(wrapper, cohort)
                cohorts.append(cohort)
                for line in (load / "requests.jsonl").read_text().splitlines():
                    row = json.loads(line)
                    if "output_file" in row:
                        row["output_file"] = str((load / row["output_file"]).relative_to(directory))
                    records.append(row)
                print(f"cpu-{budget} {engine} repetition {repetition}: {len(cohort['selected_cases'])} timed requests completed", flush=True)
        summary = summarize(records, cohorts, list(ENGINES), SimpleNamespace(cpus=cpus, whitespace="collapse", seed=args.seed))
        summary["enforcement"] = ENFORCEMENT
        summary["cpu_policy"] = args.cpu_policy
        summary["cpu_policy_description"] = CPU_POLICIES[args.cpu_policy]
        summary.pop("affinity_escapes", None)  # CPU-set proof is cgroup enforcement, not native PID sampling.
        summary["cohorts"] = cohorts
        failures.extend(f"cpu-{budget}: {error}" for error in check_summary(summary, args.repeats))
        save_json(directory / "summary.json", summary)
        save_json(directory / "cohorts.json", cohorts)
        (directory / "requests.jsonl").write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in records))
        summaries[str(budget)] = summary
    save_json(args.output / "summary.json", summaries)
    save_json(args.output / "gate.json", {"status": "failed" if failures else "passed", "errors": failures,
              "cpu_policy": args.cpu_policy,
              "timed_requests": 15 * args.repeats * 2 * 2, "performance_threshold": None,
              "accuracy_smoke_cer_ceiling": {"tika-indra": .01, "tika-tesseract-best": .05}})
    lines = [f"# Signed Docker benchmark: {args.cpu_policy} CPU policy", "",
             "Actual image measurements under one aggregate application budget, with cgroup v2 accounting.", "",
             CPU_POLICIES[args.cpu_policy], "",
             "| Total application CPUs | Engine | Timed requests | Pages/s | p50 ms | CER |", "|---:|---|---:|---:|---:|---:|"]
    for budget, summary in summaries.items():
        for name, item in summary["engines"].items():
            lines.append(f"| {budget} | {name} | {item['attempts']} | {item['pages_per_second']:.4f} | {item['latency_success_only_ms']['p50']:.2f} | {100 * item['accuracy']['micro_cer']:.4f}% |")
    lines.extend(["", "The baseline is the pinned tessdata_best English model. These measurements retain the final images' Java settings and use OCR-only PDF rendering at 300 DPI. CPU policies are separate experiments; requests from different policies are never pooled.",
                  "", "This is a finite, closed-loop synthetic integration corpus. Timing repeats do not increase the independent accuracy sample count. Tail percentiles are exploratory, and shared-runner throughput has no fixed pass/fail speedup threshold.",
                  "", "The application parent enforces CPU quota=100%/200%, memory=3GiB and zero swap. CPU and memory use aggregate-parent counters, so descendants are not double-counted. Memory is charged memory, not process RSS. Measured CPU deltas bracket the HTTP cohort; raw 250 ms samples also include client warmup and scoring. Container/model startup is outside client timing; parent-before/after captures whole-container-lifetime CPU and peak memory separately.",
                  "", "Indra predictions are produced with the pinned Datalab Indra model and remain subject to its output attribution/share-alike terms; see [the accompanying model license](INDRA-MODEL-LICENSE.txt). Code licensing is separate.", ""])
    (args.output / "report.md").write_text("\n".join(lines))
    return int(bool(failures))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    gate = commands.add_parser("matrix")
    gate.add_argument("--tika-ref", type=image_reference, required=True)
    gate.add_argument("--backend-ref", type=image_reference, required=True)
    gate.add_argument("--baseline-ref", type=image_reference, required=True)
    gate.add_argument("--identity", required=True)
    gate.add_argument("--output", type=Path, required=True)
    gate.add_argument("--cpu-policy", choices=("shared", "deployment"), default="shared")
    gate.add_argument("--repeats", type=int, choices=(1, 2, 3), default=3)
    gate.add_argument("--seed", type=int, default=20261017)
    load = commands.add_parser("load")
    load.add_argument("--wrapper-output", type=Path, required=True)
    load.add_argument("--parent-cgroup-path", type=Path, required=True)
    load.add_argument("--output", type=Path, required=True)
    load.add_argument("--engine", choices=ENGINES, required=True)
    load.add_argument("--budget", type=int, choices=(1, 2), required=True)
    load.add_argument("--cpu-policy", choices=("shared", "deployment"), default="shared")
    load.add_argument("--cpus", required=True)
    load.add_argument("--driver-cpu", type=int, required=True)
    load.add_argument("--repetition", type=int, choices=range(3), required=True)
    load.add_argument("--seed", type=int, required=True)
    load.add_argument("--url", required=True)
    args = parser.parse_args()
    if args.command == "load":
        return load_cohort(args)
    def cancelled(signum, frame):
        raise KeyboardInterrupt(f"benchmark received signal {signum}")
    signal.signal(signal.SIGTERM, cancelled)
    try:
        return matrix(args)
    except BaseException as exc:
        if getattr(args, "owned_output", False):
            save_json(args.output / "gate.json", {"status": "failed", "cpu_policy": args.cpu_policy,
                                                 "error": f"{type(exc).__name__}: {exc}"})
        raise
    finally:
        if getattr(args, "owned_output", False):
            save_json(args.output / "sha256.json", {str(path.relative_to(args.output)): digest(path)
                      for path in sorted(args.output.rglob("*")) if path.is_file() and path != args.output / "sha256.json"})


if __name__ == "__main__":
    raise SystemExit(main())
