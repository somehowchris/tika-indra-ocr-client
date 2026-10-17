#!/usr/bin/env python3
"""Fetch three pinned upstream regression fixtures without redistributing them."""

import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    lock_path = ROOT / "benchmarks/public-regression.lock.json"
    manifest_bytes = lock_path.read_bytes()
    manifest = json.loads(manifest_bytes)
    args.output.mkdir(parents=True, exist_ok=False)
    for item in manifest["files"]:
        if Path(item["name"]).name != item["name"]:
            raise ValueError("fixture name must be a basename")
        with urllib.request.urlopen(item["url"], timeout=30) as response:
            body = response.read(8 * 1024 * 1024 + 1)
        if len(body) != item["bytes"] or hashlib.sha256(body).hexdigest() != item["sha256"]:
            raise ValueError(f"fixture integrity mismatch: {item['name']}")
        (args.output / item["name"]).write_bytes(body)
    (args.output / "manifest.json").write_bytes(manifest_bytes)
    print(args.output / "manifest.json")


if __name__ == "__main__":
    main()
