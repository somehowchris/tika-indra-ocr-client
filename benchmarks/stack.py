#!/usr/bin/env python3
"""Run benchmark services in one inherited process group and CPU affinity set."""

import argparse
import json
import os
import signal
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    args = parser.parse_args()
    services = json.loads(args.plan.read_text())["services"]
    stopped = threading.Event()
    processes = []
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda _signal, _frame: stopped.set())
    try:
        for service in services:
            proc = subprocess.Popen(service["command"], env=os.environ | service.get("env", {}))
            processes.append(proc)
            deadline = time.monotonic() + service.get("ready_timeout_seconds", 120)
            while not stopped.is_set():
                if proc.poll() is not None:
                    raise RuntimeError(f"service exited before readiness: {service['name']}={proc.returncode}")
                try:
                    with urllib.request.urlopen(service["readiness_url"], timeout=.5) as response:
                        if response.status == 200:
                            break
                except (urllib.error.URLError, TimeoutError):
                    pass
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"readiness deadline: {service['name']}")
                stopped.wait(.1)
            if stopped.is_set():
                return 1
        while not stopped.wait(.1):
            if any(proc.poll() is not None for proc in processes):
                raise RuntimeError("a benchmark service exited unexpectedly")
        return 0
    finally:
        for proc in reversed(processes):
            if proc.poll() is None:
                proc.terminate()
        for proc in reversed(processes):
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=2)


if __name__ == "__main__":
    raise SystemExit(main())
