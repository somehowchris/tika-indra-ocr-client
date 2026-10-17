"""Verify the exact local model snapshot; this module never uses the network."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re


def read_manifest(path: Path) -> dict:
    lock = json.loads(path.read_text(encoding="utf-8"))
    if lock["repository"] != "datalab-to/indra-ocr" or not re.fullmatch(r"[0-9a-f]{40}", lock["revision"]):
        raise ValueError("model manifest requires an immutable upstream commit")
    names: set[str] = set()
    for item in lock["files"]:
        name = item["name"]
        if Path(name).name != name or name in names:
            raise ValueError("invalid or duplicate model filename")
        if not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) or item["size"] <= 0:
            raise ValueError(f"invalid model checksum or size: {name}")
        names.add(name)
    return lock


def verify_models(directory: Path, manifest: Path, include_math: bool) -> dict:
    lock = read_manifest(manifest)
    for item in lock["files"]:
        if item["profile"] == "math" and not include_math:
            continue
        path = directory / item["name"]
        if not path.is_file() or path.is_symlink() or path.stat().st_size != item["size"]:
            raise ValueError(f"missing or wrong-sized model file: {item['name']}")
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != item["sha256"]:
            raise ValueError(f"model checksum mismatch: {item['name']}")
    allowed = {item["name"] for item in lock["files"]}
    unexpected = {path.name for path in directory.glob("*.onnx")} - allowed
    if unexpected:
        raise ValueError(f"unlocked ONNX models present: {', '.join(sorted(unexpected))}")
    return lock
