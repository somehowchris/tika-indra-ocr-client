#!/usr/bin/env python3
"""Bounded, manifest-driven OCR experiments. See benchmarks/README.md."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import mimetypes
import os
import platform
import random
import re
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from benchmarks.metrics import aggregate, paired_cer_interval, percentile, score
from benchmarks.procstats import TreeMonitor, cpu_set


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def save_json(path: Path, data) -> None:
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def load_cases(manifest: Path, maximum: int, max_bytes: int) -> tuple[dict, list[dict]]:
    data = read_json(manifest)
    if data.get("schema_version") != 1 or not isinstance(data.get("cases"), list):
        raise ValueError("manifest requires schema_version: 1 and a cases array")
    if not data["cases"]:
        raise ValueError("manifest has no cases")
    cases = []
    seen = set()
    for original in data["cases"][:maximum]:
        case = dict(original)
        case_id = case.get("id", "")
        if not re.fullmatch(r"[A-Za-z0-9_-]+", case_id) or case_id in seen:
            raise ValueError(f"invalid or duplicate case ID: {case_id!r}")
        seen.add(case_id)
        for key in ("input", "truth"):
            path = (manifest.parent / case[key]).resolve()
            if not path.is_file() or path.stat().st_size > max_bytes:
                raise ValueError(f"missing or oversized {key} for {case_id}: {path}")
            actual = digest(path)
            if case.get(key + "_sha256") not in (None, actual):
                raise ValueError(f"SHA-256 mismatch: {case_id}/{key}")
            case[key] = str(path)
            case[key + "_sha256"] = actual
        case["reference"] = Path(case["truth"]).read_text(encoding="utf-8")
        case.setdefault("document_id", case_id)
        case.setdefault("media_type", mimetypes.guess_type(case["input"])[0] or "application/octet-stream")
        case.setdefault("pages", 1)
        if not isinstance(case["pages"], int) or case["pages"] < 1:
            raise ValueError(f"invalid page count for {case_id}")
        cases.append(case)
    return data, cases


def command(argv: list[str], case: dict | None = None) -> list[str]:
    if not isinstance(argv, list) or not argv or not all(isinstance(a, str) for a in argv):
        raise ValueError("commands must be nonempty JSON argv arrays, never shell strings")
    if case is None:
        return argv
    return [arg.replace("{input}", case["input"]).replace("{case_id}", case["id"]) for arg in argv]


def attest_affinity(proc: subprocess.Popen, allowed: set[int]) -> None:
    """The taskset launcher starts on the driver CPU before applying its mask.

    Admit the process to application sampling only after that control-plane
    transition. Subsequent per-thread escapes remain hard validation errors.
    """
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            if os.sched_getaffinity(proc.pid) <= allowed:
                return
        except ProcessLookupError:
            return  # A failed/very short command is still accounted by wait4.
        time.sleep(.001)
    raise RuntimeError("launcher did not establish the required CPU affinity")


def terminate(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait(timeout=3)
    # A server may exit before its children; reap the whole original group.
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def wait_usage(proc: subprocess.Popen, timeout: float) -> tuple[int, dict]:
    """wait4 captures command CPU, including descendants it waited for."""
    deadline = time.monotonic() + timeout
    while True:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            proc.returncode = os.waitstatus_to_exitcode(status)
            return proc.returncode, {
                "wait4_cpu_seconds": usage.ru_utime + usage.ru_stime,
                "wait4_max_process_rss_bytes": usage.ru_maxrss * 1024,
                "wait4_voluntary_context_switches": usage.ru_nvcsw,
                "wait4_involuntary_context_switches": usage.ru_nivcsw,
            }
        if time.monotonic() >= deadline:
            terminate(proc)
            raise TimeoutError(f"command deadline {timeout:.3f}s exceeded")
        time.sleep(min(0.005, max(0, deadline - time.monotonic())))


def run_case(engine: dict, case: dict, args, directory: Path,
             monitor: TreeMonitor, deadline: float, repetition: int, phase: str) -> dict:
    base = directory / f"{phase}-{case['id']}"
    result = {
        "engine": engine["name"], "case_id": case["id"],
        "document_id": case["document_id"], "kind": case.get("kind", "unspecified"),
        "pages": case["pages"], "repetition": repetition, "phase": phase,
        "status": "error", "latency_ms": None,
    }
    started = time.monotonic()
    remaining = min(args.timeout, deadline - started)
    if remaining <= 0:
        result["error"] = "experiment_deadline_exceeded_before_request"
        result["_text"] = ""
        return result
    try:
        if engine["mode"] == "command":
            env = os.environ | engine.get("env", {})
            argv = ["taskset", "--cpu-list", args.cpus] + command(engine["command"], case)
            with base.with_suffix(".txt").open("wb") as stdout, base.with_suffix(".stderr").open("wb") as stderr:
                proc = subprocess.Popen(argv, stdout=stdout, stderr=stderr, env=env, start_new_session=True)
                try:
                    attest_affinity(proc, cpu_set(args.cpus))
                    monitor.add(proc.pid)
                    returncode, usage = wait_usage(proc, remaining)
                finally:
                    terminate(proc)
            result.update(usage, returncode=returncode)
            if returncode:
                raise RuntimeError(f"command exited with status {returncode}; see {base.name}.stderr")
            if base.with_suffix(".txt").stat().st_size > args.max_output_bytes:
                raise ValueError("output exceeded maximum bytes")
            text = base.with_suffix(".txt").read_text(encoding="utf-8")
        elif engine["mode"] == "http":
            payload = Path(case["input"]).read_bytes()
            headers = {"Content-Type": case["media_type"], "Accept": "text/plain"} | engine.get("headers", {})
            request = urllib.request.Request(engine["url"], data=payload,
                                             headers=headers, method=engine.get("method", "PUT"))
            with urllib.request.urlopen(request, timeout=remaining) as response:
                result["http_status"] = response.status
                output = response.read(args.max_output_bytes + 1)
                if len(output) > args.max_output_bytes:
                    raise ValueError("output exceeded maximum bytes")
                text = output.decode("utf-8")
            base.with_suffix(".txt").write_text(text, encoding="utf-8")
        else:
            raise ValueError(f"unknown engine mode: {engine['mode']}")
        result["status"] = "ok"
        result["_text"] = text
        result["output_file"] = str(base.with_suffix(".txt").relative_to(args.output))
    except (OSError, ValueError, RuntimeError, TimeoutError) as exc:
        result["error"] = f"{type(exc).__name__}: {exc}"
        result["_text"] = ""
    result["latency_ms"] = (time.monotonic() - started) * 1000
    return result


def score_result(result: dict, case: dict, args) -> None:
    """Run scoring after the timed workload so it cannot depress throughput."""
    text = result.pop("_text", "")
    if result["status"] == "ok":
        result["output_sha256"] = hashlib.sha256(text.encode()).hexdigest()
    result["score"] = score(case["reference"], text, args.whitespace)
    result["strict_score"] = score(case["reference"], text, "preserve")


def start_server(engine: dict, args, directory: Path, monitor: TreeMonitor, deadline: float) -> tuple[subprocess.Popen | None, float]:
    if time.monotonic() >= deadline:
        raise TimeoutError("experiment deadline exceeded before service startup")
    if engine["mode"] != "http":
        return None, 0.0
    if "server_command" not in engine:
        raise ValueError("HTTP engine requires server_command so the runner can constrain and observe every process")
    # Refuse a stale listener: otherwise readiness could belong to another stack.
    readiness = engine.get("readiness_url", engine["url"])
    try:
        with urllib.request.urlopen(readiness, timeout=0.25):
            raise ValueError(f"readiness URL is already served before launch: {readiness}")
    except urllib.error.HTTPError as exc:
        raise ValueError(f"readiness URL already responds HTTP {exc.code}: {readiness}") from exc
    except (urllib.error.URLError, TimeoutError):
        pass
    logfile = (directory / "server.log").open("wb")
    started = time.monotonic()
    try:
        proc = subprocess.Popen(
            ["taskset", "--cpu-list", args.cpus] + command(engine["server_command"]),
            stdout=logfile, stderr=subprocess.STDOUT,
            env=os.environ | engine.get("env", {}), start_new_session=True,
        )
    finally:
        logfile.close()
    try:
        attest_affinity(proc, cpu_set(args.cpus))
        monitor.add(proc.pid)
        while time.monotonic() < min(deadline, started + engine.get("ready_timeout_seconds", 120)):
            if proc.poll() is not None:
                raise RuntimeError(f"server exited {proc.returncode}; see {directory / 'server.log'}")
            try:
                with urllib.request.urlopen(readiness, timeout=0.5) as response:
                    if response.status == 200:
                        return proc, (time.monotonic() - started) * 1000
            except (urllib.error.URLError, TimeoutError):
                pass
            time.sleep(0.1)
        raise TimeoutError("server readiness deadline exceeded")
    except BaseException:
        terminate(proc)
        raise


def backend_metrics(path: Path) -> dict:
    events = []
    if path.exists():
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if isinstance(row, dict) and row.get("event") == "ocr_request":
                events.append(row)
    hist = {}
    padded_pixels = valid_pixels = 0
    for event in events:
        for batch in event.get("batches", []):
            stage, size, calls = batch["stage"], batch["batch"], batch["calls"]
            key = f"{stage}:B{size}"
            hist[key] = hist.get(key, 0) + calls
            padded_pixels += size * batch["height"] * batch["width"] * calls
            valid_pixels += batch["valid_columns"] * batch["height"] * calls
    return {"request_count_including_warmup": len(events), "events": events,
            "actual_tensor_batch_histogram": hist, "padded_pixels": padded_pixels,
            "valid_pixels": valid_pixels,
            "padding_fraction": 1 - valid_pixels / padded_pixels if padded_pixels else None}


def host_metadata() -> dict:
    result = {"platform": platform.platform(), "machine": platform.machine(),
              "python": sys.version, "visible_cpu_count": os.cpu_count(),
              "initial_affinity": sorted(os.sched_getaffinity(0))}
    for path in ("/proc/cpuinfo", "/proc/self/cgroup", "/sys/fs/cgroup/cpu.max",
                 "/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/cpuset.cpus.effective"):
        try:
            text = Path(path).read_text()
            if path.endswith("cpuinfo"):
                text = "\n".join(line for line in text.splitlines()[:40]
                                 if line.startswith(("model name", "vendor_id", "flags", "cpu MHz")))
            result[path] = text.strip()
        except OSError:
            result[path] = None
    return result


def summarize(records: list[dict], cohorts: list[dict], names: list[str], args) -> dict:
    summary = {"engines": {}, "comparisons": [], "budget_logical_cpus": sorted(cpu_set(args.cpus)),
               "enforcement": "shared inherited Linux CPU affinity; not an isolated cgroup CPU quota",
               "accuracy_definition": {"units": "NFC Unicode codepoints, NOT grapheme clusters",
                                       "whitespace": args.whitespace, "case": "preserved", "punctuation": "preserved",
                                       "denominator": "ground-truth units; CER/WER may exceed 1",
                                       "repetitions": "only repetition 0 measured output counts toward accuracy"}}
    unique_by_engine = {}
    for name in names:
        rows = [r for r in records if r["engine"] == name and r["phase"] == "measured"]
        successes = [r for r in rows if r["status"] == "ok"]
        durations = [r["latency_ms"] for r in successes]
        accuracy = [dict(r["score"], case_id=r["case_id"], document_id=r["document_id"])
                    for r in rows if r["repetition"] == 0 and "score" in r]
        unique_by_engine[name] = accuracy
        output_hashes = {}
        for row in successes:
            output_hashes.setdefault(row["case_id"], set()).add(row["output_sha256"])
        groups = [c for c in cohorts if c["engine"] == name]
        makespan = sum(c.get("measured_wall_seconds", 0) for c in groups)
        pages = sum(r["pages"] for r in successes)
        item = {
            "attempts": len(rows), "successes": len(successes), "failures": len(rows) - len(successes),
            "failure_rate": (len(rows) - len(successes)) / len(rows) if rows else None,
            "successful_pages": pages, "measured_wall_seconds": makespan,
            "pages_per_second": pages / makespan if makespan else None,
            "latency_success_only_ms": {"p50": percentile(durations, .50), "p95": percentile(durations, .95),
                                        "p99": percentile(durations, .99), "max": max(durations) if durations else None},
            "accuracy": aggregate(accuracy),
            "strict_accuracy": aggregate([r["strict_score"] for r in rows
                                          if r["repetition"] == 0 and "strict_score" in r]),
            "nondeterministic_output_case_ids": sorted(k for k, v in output_hashes.items() if len(v) > 1),
            "startup_ms_each_repetition": [c.get("startup_ms") for c in groups],
            "warmup_first_request_ms_each_repetition": [c.get("first_request_ms") for c in groups],
            "accuracy_by_kind": {},
        }
        for kind in sorted({r["kind"] for r in rows}):
            item["accuracy_by_kind"][kind] = aggregate([r["score"] for r in rows
                                                      if r["kind"] == kind and r["repetition"] == 0 and "score" in r])
        if len(durations) < 1000:
            item["tail_warning"] = "fewer than 1000 successful requests: p95/p99 are exploratory, not a tail-latency guarantee"
        summary["engines"][name] = item
    if names:
        for name in names[1:]:
            summary["comparisons"].append({"baseline": names[0], "candidate": name,
                **paired_cer_interval(unique_by_engine[names[0]], unique_by_engine[name], args.seed)})
    summary["affinity_escapes"] = [dict(engine=c["engine"], repetition=c["repetition"], escapes=c["resources"]["affinity_escapes"])
                                  for c in cohorts if c.get("resources", {}).get("affinity_escapes")]
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cpus", required=True, help="shared application logical CPU list, width 1 or 2")
    parser.add_argument("--driver-cpu", type=int, required=True, help="disjoint CPU for load generation and measurement")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--repetition-start", type=int, default=0,
                        help="explicit repetition index for a separately retained supplemental run")
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--concurrency", type=int, default=1)
    parser.add_argument("--max-cases", type=int, default=16)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--max-run-seconds", type=float, default=1800)
    parser.add_argument("--max-input-bytes", type=int, default=64 * 1024 * 1024)
    parser.add_argument("--max-output-bytes", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--sample-interval", type=float, default=0.1)
    parser.add_argument("--seed", type=int, default=20261017)
    parser.add_argument("--whitespace", choices=("collapse", "preserve"), default="collapse")
    args = parser.parse_args(argv)
    allowed = cpu_set(args.cpus)
    available = os.sched_getaffinity(0)
    if len(allowed) not in (1, 2) or not allowed <= available:
        parser.error("application CPU set must contain 1 or 2 available logical CPUs")
    if args.driver_cpu in allowed or args.driver_cpu not in available:
        parser.error("driver CPU must be available and disjoint from the application CPU set")
    if not (1 <= args.repeats <= 10 and 1 <= args.concurrency <= 32 and 1 <= args.max_cases <= 10000):
        parser.error("repeats=1..10, concurrency=1..32, max-cases=1..10000 required")
    if not (0 <= args.repetition_start <= 9 and args.repetition_start + args.repeats <= 10):
        parser.error("repetition indices must remain within 0..9")
    if args.warmup < 0 or args.timeout <= 0 or args.max_run_seconds <= 0 or args.sample_interval < 0.02:
        parser.error("warmup must be nonnegative; deadlines positive; sampling interval >= 0.02")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest, cases = load_cases(args.manifest.resolve(), args.max_cases, args.max_input_bytes)
    plan = read_json(args.plan)
    engines = plan["engines"]
    names = [e["name"] for e in engines]
    if not engines or len(set(names)) != len(names) or any(not re.fullmatch(r"[A-Za-z0-9_-]+", n) for n in names):
        raise ValueError("engine names must be unique safe identifiers")
    metadata = {"started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "arguments": {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
                "host": host_metadata(), "plan": plan, "manifest_sha256": digest(args.manifest),
                "plan_sha256": digest(args.plan), "synthetic": manifest.get("synthetic", False),
                "cases": [{k: v for k, v in c.items() if k != "reference"} for c in cases],
                "workload_model": "closed-loop bounded corpus makespan; not open-loop SLO proof"}
    save_json(args.output / "environment.json", metadata)
    os.sched_setaffinity(0, {args.driver_cpu})
    records, cohorts = [], []
    base_order = engines[:]
    random.Random(args.seed).shuffle(base_order)
    deadline = time.monotonic() + args.max_run_seconds
    with (args.output / "requests.jsonl").open("w", encoding="utf-8") as stream:
        for repetition in range(args.repetition_start, args.repetition_start + args.repeats):
            order = base_order[repetition % len(engines):] + base_order[:repetition % len(engines)]
            for engine in order:
                directory = args.output / f"{engine['name']}-{repetition}"
                directory.mkdir()
                selected = [c for c in cases if c["media_type"] in engine.get("media_types", [c["media_type"]])]
                random.Random(args.seed + repetition).shuffle(selected)
                if not selected:
                    raise ValueError(f"no matching cases for {engine['name']}")
                cohort = {"engine": engine["name"], "repetition": repetition,
                          "concurrency": args.concurrency, "selected_cases": [c["id"] for c in selected]}
                monitor = TreeMonitor(allowed, args.sample_interval)
                monitor.start()
                server = None
                try:
                    server, cohort["startup_ms"] = start_server(engine, args, directory, monitor, deadline)
                    for i in range(args.warmup):
                        case = selected[i % len(selected)]
                        row = run_case(engine, case, args, directory, monitor, deadline, repetition, f"warmup{i}")
                        if i == 0:
                            cohort["first_request_ms"] = row["latency_ms"]
                        score_result(row, case, args)
                        records.append(row)
                        stream.write(json.dumps(row, ensure_ascii=False) + "\n")
                        stream.flush()
                        if row["status"] != "ok":
                            cohort.setdefault("warmup_errors", []).append(row.get("error"))
                    measured_rows = []
                    started = time.monotonic()
                    with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
                        futures = [pool.submit(run_case, engine, c, args, directory, monitor,
                                               deadline, repetition, "measured") for c in selected]
                        for future in concurrent.futures.as_completed(futures):
                            row = future.result()
                            measured_rows.append(row)
                    cohort["measured_wall_seconds"] = time.monotonic() - started
                    by_id = {c["id"]: c for c in selected}
                    for row in measured_rows:
                        score_result(row, by_id[row["case_id"]], args)
                        records.append(row)
                        stream.write(json.dumps(row, ensure_ascii=False) + "\n")
                        stream.flush()
                except (OSError, ValueError, RuntimeError, TimeoutError) as exc:
                    cohort["startup_or_harness_error"] = f"{type(exc).__name__}: {exc}"
                    for case in selected:
                        if not any(r["engine"] == engine["name"] and r["repetition"] == repetition
                                   and r["case_id"] == case["id"] and r["phase"] == "measured" for r in records):
                            row = {"engine": engine["name"], "repetition": repetition, "phase": "measured",
                                   "case_id": case["id"], "document_id": case["document_id"], "kind": case.get("kind", "unspecified"),
                                   "pages": case["pages"], "status": "error", "latency_ms": None,
                                   "error": cohort["startup_or_harness_error"],
                                   "score": score(case["reference"], "", args.whitespace),
                                   "strict_score": score(case["reference"], "", "preserve")}
                            records.append(row)
                            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
                            stream.flush()
                finally:
                    if server:
                        terminate(server)
                    cohort["resources"] = monitor.finish()
                    cohort["backend_metrics"] = backend_metrics(directory / "server.log")
                    if engine.get("assert_no_indra_ocr") and cohort["backend_metrics"]["request_count_including_warmup"]:
                        cohort["validation_error"] = "native AUTO control invoked Indra OCR"
                    cohorts.append(cohort)
                    save_json(directory / "resources.json", cohort)
    summary = summarize(records, cohorts, names, args)
    save_json(args.output / "summary.json", summary)
    save_json(args.output / "cohorts.json", cohorts)
    print(json.dumps({"output": str(args.output), "engines": {name: {key: value[key] for key in ("successes", "failures", "pages_per_second", "latency_success_only_ms", "accuracy")} for name, value in summary["engines"].items()}}, indent=2))
    return 1 if summary["affinity_escapes"] or any(c.get("validation_error") for c in cohorts) or any(e["failures"] for e in summary["engines"].values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
