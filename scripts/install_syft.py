#!/usr/bin/env python3
"""Install one checksum-pinned Linux/amd64 Syft executable, without an image."""

import argparse
import hashlib
import io
from pathlib import Path
import tarfile
import urllib.request

VERSION = "1.54.1"
SHA256 = "c069905b391cc4c20a5ba65ad5c10be2a7ba074f8ea6ad203e24d14e303dad47"
URL = f"https://github.com/anchore/syft/releases/download/v{VERSION}/syft_{VERSION}_linux_amd64.tar.gz"
parser = argparse.ArgumentParser()
parser.add_argument("--directory", type=Path, required=True)
args = parser.parse_args()
with urllib.request.urlopen(URL, timeout=60) as response:
    archive = response.read(100 * 1024 * 1024)
if hashlib.sha256(archive).hexdigest() != SHA256:
    raise SystemExit("Syft archive checksum mismatch")
with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
    member = tar.getmember("syft")
    if not member.isfile():
        raise SystemExit("Syft archive has no regular executable")
    stream = tar.extractfile(member)
    if stream is None:
        raise SystemExit("could not read Syft executable")
    args.directory.mkdir(parents=True, exist_ok=True)
    target = args.directory / "syft"
    target.write_bytes(stream.read())
    target.chmod(0o755)
print(target)
