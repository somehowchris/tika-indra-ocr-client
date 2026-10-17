#!/usr/bin/env python3
"""Run real backend contract and recovery suites with managed local processes."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--port", type=int, default=18081)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    env = os.environ | {
        "INDRA_MODEL_DIR": str(args.models.resolve()),
        "INDRA_TEST_MODEL_DIR": str(args.models.resolve()),
        "INDRA_TEST_LOG_DIR": str((args.output / "recovery").resolve()),
        "INDRA_HOST": "127.0.0.1", "INDRA_PORT": str(args.port),
        "INDRA_WORKERS": "1", "INDRA_THREADS": "1", "INDRA_MATH": "0",
        "INDRA_TEST_URL": f"http://127.0.0.1:{args.port}",
    }
    results = []
    with (args.output / "server.log").open("wb") as log:
        server = subprocess.Popen([sys.executable, str(ROOT / "backend/server.py")], env=env, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 120
            while True:
                if server.poll() is not None:
                    raise RuntimeError(f"backend exited before readiness: {server.returncode}")
                try:
                    with urllib.request.urlopen(env["INDRA_TEST_URL"] + "/readyz", timeout=1) as response:
                        if response.status == 200:
                            break
                except (urllib.error.URLError, TimeoutError):
                    pass
                if time.monotonic() >= deadline:
                    raise TimeoutError("backend startup exceeded 120 seconds")
                time.sleep(0.1)
            for pattern in ("test_server.py", "test_recovery.py"):
                command = [sys.executable, "-m", "unittest", "discover", "-s", "backend", "-p", pattern, "-v"]
                result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True, timeout=180)
                results.append({"command": command, "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
                print(result.stdout, end="")
                print(result.stderr, end="", file=sys.stderr)
                if result.returncode:
                    break
        finally:
            if server.poll() is None:
                server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=10)
            (args.output / "tests.json").write_text(json.dumps(results, indent=2) + "\n")
    return 1 if not results or any(item["exit_code"] for item in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
