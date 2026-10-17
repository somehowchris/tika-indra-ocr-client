#!/usr/bin/env python3
"""Build-time model acquisition. Runtime verification lives in model_files.py."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import urllib.request

from model_files import read_manifest, verify_models


def fetch(output: Path, manifest: Path, include_math: bool) -> None:
    lock = read_manifest(manifest)
    output.mkdir(parents=True, exist_ok=True)
    base = f"https://huggingface.co/{lock['repository']}/resolve/{lock['revision']}/"
    for item in lock["files"]:
        if item["profile"] == "math" and not include_math:
            continue
        destination = output / item["name"]
        if destination.is_file():
            with destination.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if destination.stat().st_size == item["size"] and digest == item["sha256"]:
                continue
        request = urllib.request.Request(base + item["name"], headers={"User-Agent": "tika-indra-build/1"})
        temp_name: str | None = None
        try:
            with tempfile.NamedTemporaryFile(dir=output, delete=False) as target:
                temp_name = target.name
                digest = hashlib.sha256()
                size = 0
                with urllib.request.urlopen(request, timeout=120) as response:
                    while chunk := response.read(1024 * 1024):
                        size += len(chunk)
                        if size > item["size"]:
                            raise ValueError(f"oversized model download: {item['name']}")
                        digest.update(chunk)
                        target.write(chunk)
                if size != item["size"] or digest.hexdigest() != item["sha256"]:
                    raise ValueError(f"model checksum mismatch: {item['name']}")
                target.flush()
                os.fsync(target.fileno())
            os.replace(temp_name, destination)
            temp_name = None
            destination.chmod(0o444)
        finally:
            if temp_name is not None:
                Path(temp_name).unlink(missing_ok=True)
    verify_models(output, manifest, include_math)
    print(json.dumps({"event": "models_verified", "revision": lock["revision"], "math": include_math}))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).with_name("models.lock.json"))
    parser.add_argument("--without-math", action="store_true")
    args = parser.parse_args()
    fetch(args.output, args.manifest, not args.without_math)


if __name__ == "__main__":
    main()
