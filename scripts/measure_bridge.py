#!/usr/bin/env python3
"""Measure bridge process + loopback transport + file overhead; performs NO OCR.

Linux only. The controller runs outside the application CPU set. A single-threaded
HTTP stand-in and a subprocess launcher inherit the same 1/2 logical CPU budget;
each actual bridge inherits that affinity before exec. Interleaved paired trials
reduce order drift. No privileged configuration or cgroup changes are performed.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import multiprocessing
import os
from pathlib import Path
import platform
import random
import resource
import socketserver
import statistics
import struct
import subprocess
import tempfile
import time
import tomllib
import zlib


ROOT = Path(__file__).resolve().parents[1]
REQUEST_BYTES = 256 * 1024
RESPONSE_BYTES = 16 * 1024
T95_DF2 = 4.302652729911275


def png_fixture() -> bytes:
    """Construct a valid, deterministic grayscale PNG padded to exactly 256 KiB."""
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    width = height = 511
    pixels = random.Random(1984).randbytes(width * height)
    raw = b"".join(b"\0" + pixels[row * width : (row + 1) * width] for row in range(height))
    start = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
    encoded = start + chunk(b"IDAT", zlib.compress(raw, level=0))
    end = chunk(b"IEND", b"")
    filler_size = REQUEST_BYTES - len(encoded) - len(end) - 12
    assert filler_size >= 8
    payload = encoded + chunk(b"tEXt", b"padding\0" + b"x" * (filler_size - 8)) + end
    assert len(payload) == REQUEST_BYTES
    return payload


def response_fixture() -> bytes:
    line = b"INVOICE 12345 | Example account | Item 42 | Total 987.65 USD\n"
    return (line * math.ceil(RESPONSE_BYTES / len(line)))[:RESPONSE_BYTES]


def serve_standin(pipe, cpus: list[int], payload: bytes, body: bytes) -> None:
    os.sched_setaffinity(0, cpus)
    reply = (
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: "
        + str(len(body)).encode("ascii") + b"\r\nConnection: close\r\n\r\n" + body
    )
    counts = {"requests": 0, "request_bytes": 0, "response_body_bytes": 0, "errors": 0}

    class Handler(socketserver.BaseRequestHandler):
        def handle(self) -> None:
            self.request.settimeout(5)
            try:
                with self.request.makefile("rb") as stream:
                    if stream.readline(16384) != b"POST /ocr HTTP/1.1\r\n":
                        raise ValueError("unexpected request line")
                    length = None
                    headers = 0
                    while True:
                        line = stream.readline(16384)
                        headers += len(line)
                        if headers > 16384 or not line:
                            raise ValueError("invalid headers")
                        if line == b"\r\n":
                            break
                        if line.lower().startswith(b"content-length:"):
                            if length is not None:
                                raise ValueError("duplicate length")
                            length = int(line.split(b":", 1)[1])
                    if length != len(payload) or stream.read(length) != payload:
                        raise ValueError("incorrect request body")
                    self.request.sendall(reply)
                    counts["requests"] += 1
                    counts["request_bytes"] += length
                    counts["response_body_bytes"] += len(body)
            except (OSError, ValueError) as exc:
                counts["errors"] += 1
                pipe.send({"unexpected_error": str(exc)})

    with socketserver.TCPServer(("127.0.0.1", 0), Handler) as server:
        server.timeout = 0.1
        pipe.send({"address": f"127.0.0.1:{server.server_address[1]}",
                   "pid": os.getpid(), "affinity": sorted(os.sched_getaffinity(0))})
        while not pipe.poll():
            server.handle_request()
        pipe.recv()
        usage = resource.getrusage(resource.RUSAGE_SELF)
        pipe.send({**counts, "affinity_end": sorted(os.sched_getaffinity(0)),
                   "cpu_user_seconds": usage.ru_utime, "cpu_system_seconds": usage.ru_stime})


def launch_bridges(pipe, cpus: list[int], binaries: dict[str, str], address: str,
                   input_file: str, output_base: str) -> None:
    os.sched_setaffinity(0, cpus)
    environment = dict(os.environ)
    environment.update(INDRA_ADDR=address, INDRA_TIMEOUT_SECONDS="5", INDRA_CONNECT_TIMEOUT_MS="1000")
    pipe.send({"pid": os.getpid(), "affinity": sorted(os.sched_getaffinity(0))})
    while (profile := pipe.recv()) is not None:
        command = [binaries[profile], input_file, output_base, "--psm", "3", "-c", "page_separator=",
                   "-c", "preserve_interword_spaces=0", "txt"]
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        started = time.perf_counter_ns()
        result = subprocess.run(command, env=environment, capture_output=True, timeout=10, check=False)
        elapsed = time.perf_counter_ns() - started
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        pipe.send({"wall_ns": elapsed, "bridge_user_ns": round((after.ru_utime - before.ru_utime) * 1e9),
                   "bridge_system_ns": round((after.ru_stime - before.ru_stime) * 1e9),
                   "exit_code": result.returncode, "stdout_bytes": len(result.stdout),
                   "stderr": result.stderr.decode("utf-8", errors="replace")})
    pipe.send({"affinity_end": sorted(os.sched_getaffinity(0))})


def receive(pipe, process, seconds: float = 15) -> dict:
    if not pipe.poll(seconds):
        raise RuntimeError(f"process {process.pid} did not respond; exit={process.exitcode}")
    value = pipe.recv()
    if "unexpected_error" in value:
        raise RuntimeError(value["unexpected_error"])
    return value


def terminate(process) -> None:
    if process is None:
        return
    process.join(timeout=2)
    if process.is_alive():
        process.terminate()
        process.join(timeout=2)
    if process.is_alive():
        process.kill()
        process.join(timeout=2)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = math.floor(position)
    upper = math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def replicate_interval(values: list[float]) -> dict:
    assert len(values) == 3, "This harness uses a Student t interval with 2 degrees of freedom."
    mean = statistics.mean(values)
    half = T95_DF2 * statistics.stdev(values) / math.sqrt(len(values))
    return {"estimate": mean, "lower": mean - half, "upper": mean + half,
            "method": "two-sided 95% Student t interval over 3 replication means; df=2"}


def cpu_information(cpus: list[int]) -> dict:
    cpuinfo = Path("/proc/cpuinfo").read_text(encoding="utf-8")
    model = next((line.split(":", 1)[1].strip() for line in cpuinfo.splitlines()
                  if line.startswith("model name")), "unknown")
    topology = {}
    for cpu in cpus:
        directory = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology")
        topology[str(cpu)] = {name: (directory / name).read_text().strip()
                              for name in ("core_id", "physical_package_id", "thread_siblings_list")
                              if (directory / name).exists()}
    quota_file = Path("/sys/fs/cgroup/cpu.max")
    return {"platform": platform.platform(), "python": platform.python_version(), "cpu_model": model,
            "logical_cpus_online": os.cpu_count(), "cpu_topology": topology,
            "visible_root_cpu_max": quota_file.read_text().strip() if quota_file.exists() else None,
            "cpu_enforcement": "sched_setaffinity; no CFS quota or container measurements"}


def summarize(samples: list[dict]) -> dict:
    report = {}
    for budget in (1, 2):
        profiles = {}
        for profile in ("release", "maxperf"):
            chosen = [s for s in samples if s["cpu_budget"] == budget and s["profile"] == profile]
            wall = [s["wall_ns"] / 1e6 for s in chosen]
            means = [statistics.mean(s["wall_ns"] / 1e6 for s in chosen if s["replication"] == rep)
                     for rep in (1, 2, 3)]
            profiles[profile] = {
                "requests": len(chosen), "errors": sum(s["exit_code"] != 0 for s in chosen),
                "wall_ms": {"mean": statistics.mean(wall), "p50": percentile(wall, .5),
                            "p95": percentile(wall, .95), "min": min(wall), "max": max(wall)},
                "replication_mean_ms": means, "mean_ms_interval": replicate_interval(means),
                "bridge_cpu_ms_mean": statistics.mean(
                    (s["bridge_user_ns"] + s["bridge_system_ns"]) / 1e6 for s in chosen),
            }
        release = profiles["release"]["replication_mean_ms"]
        maxperf = profiles["maxperf"]["replication_mean_ms"]
        report[str(budget)] = {
            "profiles": profiles,
            "paired_mean_difference_ms": replicate_interval([b - a for a, b in zip(release, maxperf)]),
            "paired_mean_change_percent": replicate_interval([(b / a - 1) * 100 for a, b in zip(release, maxperf)]),
            "difference_direction": "maxperf minus release; negative favors maxperf",
        }
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, default=ROOT / "target/x86_64-unknown-linux-gnu/release/indra-tesseract")
    parser.add_argument("--maxperf", type=Path, default=ROOT / "target/x86_64-unknown-linux-gnu/maxperf/indra-tesseract")
    parser.add_argument("--cpu-one", type=int, default=6)
    parser.add_argument("--cpu-two", type=int, default=7)
    parser.add_argument("--controller-cpu", type=int, default=0)
    parser.add_argument("--requests", type=int, default=100)
    parser.add_argument("--warmups", type=int, default=10)
    parser.add_argument("--output", type=Path, default=ROOT / "validation")
    args = parser.parse_args()
    if args.requests <= 0 or args.requests % 2 or args.warmups < 0:
        parser.error("--requests must be a positive even number and --warmups nonnegative")
    if len({args.cpu_one, args.cpu_two, args.controller_cpu}) != 3:
        parser.error("application CPUs and controller CPU must be distinct")
    initial_affinity = sorted(os.sched_getaffinity(0))
    cpus = [args.cpu_one, args.cpu_two, args.controller_cpu]
    if not set(cpus).issubset(initial_affinity):
        parser.error(f"requested CPUs are outside allowed affinity {initial_affinity}")
    binaries = {"release": str(args.release.resolve(strict=True)), "maxperf": str(args.maxperf.resolve(strict=True))}
    binary_info = {name: {"path": path, "bytes": Path(path).stat().st_size,
                          "sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest()}
                   for name, path in binaries.items()}
    cargo_bytes = (ROOT / "Cargo.toml").read_bytes()
    cargo = tomllib.loads(cargo_bytes.decode("utf-8"))
    metadata = {"schema_version": 1, "experiment": "bridge profile loopback HTTP stand-in; NO OCR",
                "started_at_utc": datetime.now(timezone.utc).isoformat(), "host": cpu_information(cpus),
                "initial_allowed_affinity": initial_affinity, "controller_cpu": args.controller_cpu,
                "build_configuration": {"cargo_toml_sha256": hashlib.sha256(cargo_bytes).hexdigest(),
                    "profiles": {name: cargo["profile"][name] for name in ("release", "maxperf")},
                    "target": "x86_64-unknown-linux-gnu", "provenance": "prebuilt binaries supplied by project owner; this script does not compile"},
                "binaries": binary_info, "replications": 3, "requests_per_profile_per_replication": args.requests,
                "warmups_per_profile_per_budget_per_replication_excluded": args.warmups,
                "pair_order": "50/50 AB and BA within each replication/budget; fixed-seed shuffled",
                "random_seed": 9124, "timed_scope": "Python subprocess launch to bridge exit, including loopback request/response and output file create/write/close",
                "excluded_scope": "controller IPC, stand-in startup, fixture generation, output reread/hash validation/unlink, warmups",
                "limitations": ["No OCR, Tika/JVM, image decoding, tensor batching or semantic accuracy is measured.",
                                "Serial requests only. The two-CPU budget permits stand-in and bridge overlap; it is not a parallel OCR throughput test.",
                                "Affinity limits use of logical CPUs; this does not prove CFS quota or Docker/Kubernetes enforcement.",
                                "Warm filesystem cache; no fsync or physical storage durability measurement.",
                                "Python launcher and stand-in overhead are included; this is not isolated Rust function latency or Java ProcessBuilder latency.",
                                "Shared host, uncontrolled CPU frequency and external load; 3-replication intervals are exploratory."]}
    payload, body = png_fixture(), response_fixture()
    metadata["request"] = {"bytes": len(payload), "type": "valid 511x511 grayscale PNG with deterministic random pixels and padding",
                           "sha256": hashlib.sha256(payload).hexdigest()}
    metadata["response"] = {"bytes": len(body), "type": "deterministic ASCII text/plain",
                            "sha256": hashlib.sha256(body).hexdigest()}
    os.sched_setaffinity(0, [args.controller_cpu])
    context = multiprocessing.get_context("spawn")
    rng = random.Random(9124)
    samples, runs = [], []
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="bridge-profile-", dir=args.output.parent) as temp:
        input_file = Path(temp) / "page.png"
        output_base = Path(temp) / "output"
        output_file = Path(str(output_base) + ".txt")
        input_file.write_bytes(payload)
        for replication in (1, 2, 3):
            for budget in ([1, 2] if replication % 2 else [2, 1]):
                mask = [args.cpu_one] if budget == 1 else [args.cpu_one, args.cpu_two]
                parent_server, child_server = context.Pipe()
                server = context.Process(target=serve_standin, args=(child_server, mask, payload, body))
                parent_launcher, child_launcher = context.Pipe()
                launcher = None
                server.start()
                try:
                    server_info = receive(parent_server, server)
                    launcher = context.Process(target=launch_bridges, args=(child_launcher, mask, binaries,
                        server_info["address"], str(input_file), str(output_base)))
                    launcher.start()
                    launcher_info = receive(parent_launcher, launcher)

                    def request(profile: str) -> dict:
                        parent_launcher.send(profile)
                        sample = receive(parent_launcher, launcher)
                        if sample["exit_code"] or sample["stdout_bytes"] or sample["stderr"]:
                            raise RuntimeError(f"bridge failure: {sample}")
                        if output_file.read_bytes() != body:
                            raise RuntimeError("bridge output differs from expected 16 KiB text")
                        output_file.unlink()
                        sample["validated_output_bytes"] = len(body)
                        return sample

                    for warmup in range(args.warmups):
                        for profile in (("release", "maxperf") if warmup % 2 == 0 else ("maxperf", "release")):
                            request(profile)
                    orders = [0] * (args.requests // 2) + [1] * (args.requests // 2)
                    rng.shuffle(orders)
                    for pair, order in enumerate(orders, start=1):
                        profiles = ("release", "maxperf") if order == 0 else ("maxperf", "release")
                        for position, profile in enumerate(profiles, start=1):
                            sample = request(profile)
                            samples.append({"sample": len(samples) + 1, "replication": replication,
                                            "cpu_budget": budget, "affinity": mask, "pair": pair,
                                            "position_in_pair": position, "profile": profile, **sample})
                    parent_launcher.send(None)
                    launcher_end = receive(parent_launcher, launcher)
                    parent_server.send(None)
                    server_end = receive(parent_server, server)
                    expected = 2 * (args.requests + args.warmups)
                    if server_end["requests"] != expected or server_end["errors"]:
                        raise RuntimeError(f"stand-in request accounting mismatch: {server_end}")
                    runs.append({"replication": replication, "cpu_budget": budget, "affinity": mask,
                                 "stand_in": {**server_info, **server_end}, "launcher": {**launcher_info, **launcher_end}})
                    print(json.dumps({"replication": replication, "cpu_budget": budget,
                                      "measured_requests": 2 * args.requests, "errors": 0}), flush=True)
                finally:
                    terminate(launcher)
                    terminate(server)
                    parent_launcher.close()
                    child_launcher.close()
                    parent_server.close()
                    child_server.close()
    metadata["completed_at_utc"] = datetime.now(timezone.utc).isoformat()
    metadata["controller_affinity_end"] = sorted(os.sched_getaffinity(0))
    os.sched_setaffinity(0, initial_affinity)
    args.output.mkdir(parents=True, exist_ok=True)
    full = {**metadata, "runs": runs, "samples": samples}
    summary = {**metadata, "measured_requests": len(samples), "runs": runs, "results": summarize(samples)}
    for name, data in (("bridge-profile-samples.json", full), ("bridge-profile-summary.json", summary)):
        (args.output / name).write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary["results"], indent=2))


if __name__ == "__main__":
    main()
