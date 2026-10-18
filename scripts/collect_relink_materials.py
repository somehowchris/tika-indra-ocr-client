#!/usr/bin/env python3
"""Collect source, notices and rebuild inputs for the static GNU bridge.

The caller fetches the exact glibc source through signed APT repositories first.
This collector verifies the installed package/source version and every archive
checksum in its .dsc. It performs no network access and copies no executable.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess


PROJECT_FILES = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE", "THIRD_PARTY_NOTICES.md",
    "docker/bridge.Dockerfile", "docs/relinking.md", "scripts/collect_relink_materials.py",
    "licenses/INDRA-MODEL-LICENSE.txt",
)
PACKAGE_NOTICES = (
    ("usr/share/doc/libc6/copyright", "licenses/glibc-copyright"),
    ("usr/share/common-licenses/LGPL-2.1", "licenses/LGPL-2.1"),
    ("usr/share/common-licenses/GPL-2", "licenses/GPL-2"),
    ("usr/share/common-licenses/GPL-3", "licenses/GPL-3"),
    ("usr/share/doc/libgcc-s1/copyright", "licenses/libgcc-copyright"),
)


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def ordinary_file(path: Path) -> None:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Expected a regular, non-symlink file: {path}")


def copy_file(source: Path, destination: Path) -> None:
    ordinary_file(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o644)


def copy_tree(source: Path, destination: Path) -> int:
    if source.is_symlink() or not source.is_dir():
        raise ValueError(f"Expected a non-symlink directory: {source}")
    count = 0
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Refusing a symlink in the source bundle: {path}")
        if path.is_file():
            copy_file(path, destination / path.relative_to(source))
            count += 1
        elif not path.is_dir():
            raise ValueError(f"Unexpected special file: {path}")
    return count


def deb822(path: Path) -> dict[str, str]:
    """Read one Debian source-control paragraph, including clear-signed .dsc."""
    ordinary_file(path)
    body = path.read_text(encoding="utf-8")
    if body.startswith("-----BEGIN PGP SIGNED MESSAGE-----"):
        _, separator, body = body.partition("\n\n")
        if not separator:
            raise ValueError(f"Malformed clear-signed source control: {path}")
        body = body.split("-----BEGIN PGP SIGNATURE-----", 1)[0]
        body = "\n".join(line[2:] if line.startswith("- ") else line for line in body.splitlines())
    fields: dict[str, str] = {}
    previous = None
    for line in body.splitlines():
        if not line.strip():
            continue
        if line[0].isspace():
            if previous is None:
                raise ValueError(f"Unexpected continuation in {path}")
            fields[previous] += "\n" + line.strip()
            continue
        key, separator, value = line.partition(":")
        if not separator or not re.fullmatch(r"[A-Za-z][A-Za-z0-9-]*", key) or key in fields:
            raise ValueError(f"Invalid or duplicated source-control field in {path}")
        fields[key] = value.strip()
        previous = key
    return fields


def installed_glibc(package_root: Path) -> dict[str, str]:
    command = ["dpkg-query", "--admindir=" + str(package_root / "var/lib/dpkg"), "-W",
               "-f=${source:Package}\n${source:Version}\n${Version}\n${Architecture}\n${db:Status-Status}\n", "libc6-dev"]
    values = subprocess.check_output(command, text=True).splitlines()
    if len(values) != 5 or values[0] != "glibc" or values[4] != "installed":
        raise ValueError("libc6-dev must be installed and identify its exact glibc source version")
    return dict(zip(("source_package", "source_version", "libc6_dev_version", "architecture", "status"), values))


def checked_source(directory: Path, package: dict[str, str]) -> tuple[Path, list[tuple[Path, str, int]]]:
    matches = []
    for dsc in sorted(directory.glob("*.dsc")):
        fields = deb822(dsc)
        if fields.get("Source") == package["source_package"] and fields.get("Version") == package["source_version"]:
            matches.append((dsc, fields))
    if len(matches) != 1:
        raise ValueError(f"Require exactly one glibc .dsc matching installed source version {package['source_version']}; found {len(matches)}")
    dsc, fields = matches[0]
    archives = []
    names = set()
    for line in fields.get("Checksums-Sha256", "").splitlines():
        if not line.strip():
            continue
        columns = line.split()
        if len(columns) != 3:
            raise ValueError(f"Malformed SHA-256 entry in {dsc}")
        expected, count, name = columns
        if not re.fullmatch(r"[0-9a-fA-F]{64}", expected) or not count.isdecimal():
            raise ValueError(f"Malformed source archive hash or length in {dsc}")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._+~:-]*", name) or name in names:
            raise ValueError(f"Unsafe or duplicate source archive filename: {name}")
        names.add(name)
        archive = directory / name
        ordinary_file(archive)
        size = int(count)
        if archive.stat().st_size != size or sha256(archive) != expected.lower():
            raise ValueError(f"Source archive checksum/length mismatch: {archive}")
        archives.append((archive, expected.lower(), size))
    if not archives or not any(".orig.tar." in p.name for p, _, _ in archives):
        raise ValueError("Missing upstream glibc source archive")
    if not any(".debian.tar." in p.name or p.name.endswith(".diff.gz") for p, _, _ in archives):
        raise ValueError("Missing Debian packaging/patch source archive")
    return dsc, archives


def rust_notices(sysroot: Path, output: Path) -> dict:
    documents = sysroot / "share/doc/rust"
    if not documents.is_dir():
        raise ValueError(f"Rust toolchain notice directory missing: {documents}")
    selected = [p for p in sorted(documents.iterdir())
                if p.is_file() and p.name.upper().startswith(("COPYRIGHT", "LICENSE", "LICENCE", "NOTICE"))]
    if not selected:
        raise ValueError("Rust toolchain copyright/license files are missing")
    for source in selected:
        copy_file(source, output / "licenses/rust" / source.name)
    license_count = copy_tree(documents / "licenses", output / "licenses/rust/licenses")
    if not license_count:
        raise ValueError("Rust toolchain license directory is empty")
    compiler = sysroot / "bin/rustc"
    ordinary_file(compiler)
    version = subprocess.check_output([str(compiler), "--version", "--verbose"], text=True).strip()
    return {"compiler": version, "notice_files": [p.name for p in selected],
            "additional_license_files": license_count}


def collect(args) -> dict:
    project = args.project.resolve(strict=True)
    source_directory = args.glibc_source.resolve(strict=True)
    package_root = args.package_root.resolve(strict=True)
    output = args.output.absolute()
    if output.is_symlink() or (output.exists() and any(output.iterdir())):
        raise ValueError("Output directory must be absent or empty; refusing to overwrite a bundle")
    package = installed_glibc(package_root)
    dsc, archives = checked_source(source_directory, package)
    for name in PROJECT_FILES:
        ordinary_file(project / name)
    if not (project / "src").is_dir():
        raise ValueError("Project src directory is missing")
    sysroot = (args.rust_sysroot if args.rust_sysroot is not None else
               Path(subprocess.check_output(["rustc", "--print", "sysroot"], cwd=project, text=True).strip()))
    sysroot = sysroot.resolve(strict=True)
    binary = None
    if args.binary is not None:
        ordinary_file(args.binary)
        binary = {"filename": args.binary.name, "sha256": sha256(args.binary), "bytes": args.binary.stat().st_size}
    output.mkdir(parents=True, exist_ok=True)
    for name in PROJECT_FILES:
        copy_file(project / name, output / "bridge" / name)
    copy_tree(project / "src", output / "bridge/src")
    copy_tree(project / "shared", output / "bridge/shared")
    copy_tree(project / "benches", output / "bridge/benches")
    copy_tree(project / "tests", output / "bridge/tests")
    copy_file(project / "docs/relinking.md", output / "README.md")
    copy_file(project / "THIRD_PARTY_NOTICES.md", output / "THIRD_PARTY_NOTICES.md")
    # Keep the top-level notice's local links useful outside the source tree.
    copy_file(project / "LICENSE", output / "LICENSE")
    copy_file(project / "docs/relinking.md", output / "docs/relinking.md")
    copy_file(project / "licenses/INDRA-MODEL-LICENSE.txt", output / "licenses/INDRA-MODEL-LICENSE.txt")
    copy_file(dsc, output / "glibc-source" / dsc.name)
    for archive, _, _ in archives:
        copy_file(archive, output / "glibc-source" / archive.name)
    for source, destination in PACKAGE_NOTICES:
        copy_file(package_root / source, output / destination)
    rust = rust_notices(sysroot, output)
    inventory = []
    for path in sorted(output.rglob("*")):
        if path.is_file():
            relative = str(path.relative_to(output))
            if any(c in relative for c in "\n\r\\"):
                raise ValueError(f"Unsupported checksum filename: {relative!r}")
            inventory.append({"path": relative, "bytes": path.stat().st_size, "sha256": sha256(path)})
    manifest = {
        "schema_version": 1, "purpose": "Static GNU bridge source, notices and relinking materials",
        "glibc": package, "binary": binary, "rust": rust,
        "source_control_file": dsc.name,
        "source_archive_hashes_verified": True,
        "signature_scope": "Caller fetches through signed APT metadata; collector verifies installed source version and .dsc SHA-256/lengths, not OpenPGP signatures",
        "files": inventory,
        "scope_limits": ["No executable is copied into this bundle.",
                         "Collection/hash verification does not prove a modified-glibc rebuild has been executed.",
                         "Source and notices must accompany the distributed binary/image; this inventory is not a blanket legal certification."],
    }
    manifest_file = output / "manifest.json"
    manifest_file.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    checksums = [(row["sha256"], row["path"]) for row in inventory]
    checksums.append((sha256(manifest_file), "manifest.json"))
    (output / "SHA256SUMS").write_text("".join(f"{digest}  {name}\n" for digest, name in sorted(checksums, key=lambda row: row[1])), encoding="utf-8")
    return {"output": str(output), "source_version": package["source_version"],
            "source_archives": len(archives), "files": len(inventory),
            "binary_sha256": binary["sha256"] if binary else None,
            "manifest_sha256": sha256(manifest_file)}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--glibc-source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--package-root", type=Path, default=Path("/"), help="Root containing the exact installed dpkg database and notices")
    parser.add_argument("--rust-sysroot", type=Path, help="Exact Rust toolchain; defaults to rustc --print sysroot")
    parser.add_argument("--binary", type=Path, help="Record the distributed executable's fingerprint without copying it")
    args = parser.parse_args()
    try:
        result = collect(args)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        parser.exit(1, f"Cannot collect relinking materials: {exc}\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
