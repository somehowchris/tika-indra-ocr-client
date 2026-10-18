#!/usr/bin/env python3
"""Build-only exact Cargo JSON executable selection. Never scans target directories."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil


def select(stream, package_id, name, kind, test, root):
    matches = []
    finished = []
    last_reason = None
    for number, line in enumerate(stream, 1):
        value = json.loads(line, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite Cargo JSON')))
        if not isinstance(value, dict) or not isinstance(value.get('reason'), str):
            raise ValueError(f'invalid Cargo event at line {number}')
        last_reason = value['reason']
        if value['reason'] == 'build-finished':
            if value.get('success') is not True:
                raise ValueError('Cargo did not report a successful build')
            finished.append(True)
        if value['reason'] != 'compiler-artifact' or value.get('package_id') != package_id:
            continue
        target = value.get('target')
        profile = value.get('profile')
        if not isinstance(target, dict) or not isinstance(profile, dict):
            raise ValueError('invalid selected-package target/profile')
        if target.get('name') != name or target.get('kind') != [kind] or profile.get('test') is not test:
            continue
        if value.get('executable') is not None:
            matches.append(value)
    if finished != [True] or last_reason != 'build-finished':
        raise ValueError('Cargo stream must contain exactly one successful build-finished event')
    if len(matches) != 1:
        raise ValueError(f'exactly one matching executable required; found {len(matches)}')
    event = matches[0]
    raw = event['executable']
    if not isinstance(raw, str):
        raise ValueError('executable path must be a string')
    path = Path(raw)
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError('executable must be an absolute regular executable file')
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise ValueError('artifact root must be an absolute regular directory')
    if path != path.resolve() or root != root.resolve() or not path.is_relative_to(root):
        raise ValueError('artifact executable escapes its canonical root')
    with path.open('rb') as f:
        if f.read(4) != b'\x7fELF':
            raise ValueError('selected executable is not ELF')
    filenames = event.get('filenames')
    if not isinstance(filenames, list) or not all(isinstance(item, str) for item in filenames) or raw not in filenames:
        raise ValueError('selected executable absent from Cargo filenames')
    return path, event


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', type=Path, required=True)
    parser.add_argument('--package-id', required=True)
    parser.add_argument('--name', required=True)
    parser.add_argument('--kind', choices=('lib', 'test', 'example', 'bin'), required=True)
    parser.add_argument('--test', choices=('true', 'false'), required=True)
    parser.add_argument('--artifact-root', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--receipt', type=Path, required=True)
    args = parser.parse_args()
    with args.json.open() as f:
        path, event = select(f, args.package_id, args.name, args.kind, args.test == 'true', args.artifact_root)
    if args.destination.exists() or args.destination.is_symlink() or args.receipt.exists() or args.receipt.is_symlink():
        raise ValueError('refusing to overwrite selected executable or receipt')
    with path.open('rb') as source, args.destination.open('xb') as target:
        shutil.copyfileobj(source, target)
    args.destination.chmod(path.stat().st_mode & 0o777)
    with args.destination.open('rb') as f:
        digest = hashlib.file_digest(f, 'sha256').hexdigest()
    with args.receipt.open('x') as f:
        json.dump({'schema_version': 1, 'cargo_json_sha256': hashlib.sha256(args.json.read_bytes()).hexdigest(),
                   'package_id': args.package_id, 'target': event['target'], 'profile': event['profile'],
                   'source': str(path), 'destination': str(args.destination), 'binary_sha256': digest,
                   'scope': 'selection identity only; separate readelf/static and paired byte-identity gates required'}, f, indent=2)
        f.write('\n')


if __name__ == '__main__':
    main()
