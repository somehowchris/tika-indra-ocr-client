#!/usr/bin/env python3
"""Fail unless an ELF has neither PT_INTERP nor DT_NEEDED. Static PIE is valid."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
args = parser.parse_args()
program = subprocess.run(["readelf", "--program-headers", str(args.binary)], check=True, capture_output=True, text=True).stdout
dynamic = subprocess.run(["readelf", "--dynamic", str(args.binary)], check=True, capture_output=True, text=True).stdout
if "INTERP" in program or "(NEEDED)" in dynamic:
    raise SystemExit("binary requires a dynamic loader or a shared library")
print(json.dumps({
    "binary": str(args.binary), "bytes": args.binary.stat().st_size,
    "sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
    "pt_interp": False, "dt_needed": False, "static": True,
}, indent=2))
