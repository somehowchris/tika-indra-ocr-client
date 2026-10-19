"""Reference-only build-cost reader and external phase recorder; no build recipe changes."""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import hashlib
import gzip
import json
import os
from pathlib import Path
import re
import subprocess
import signal
import time

ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
PREFIX = re.compile(r"^(?P<at>\d{4}-\d\d-\d\dT\S+Z) (?P<body>.*)$")
VERTEX = re.compile(r"^#(?P<id>\d+) (?P<body>.*)$")


def sha(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def write(path, value):
    with path.open('x') as f:
        json.dump(value, f, indent=2, allow_nan=False)
        f.write('\n')


def classify(name):
    if 'build-native-runtime.sh /native' in name:
        return 'native-build-combined'
    if 'collect-runtime-materials.sh' in name:
        return 'rust-and-materials-combined'
    if 'importing cache manifest' in name:
        return 'cache-import'
    if 'exporting cache' in name:
        return 'cache-export'
    if name.startswith('exporting to '):
        return 'image-export'
    return 'other'


def parse_log(path):
    invocation = -1
    vertices = {}
    evidence = []
    data = gzip.decompress(path.read_bytes()) if path.suffix == '.gz' else path.read_bytes()
    for number, raw in enumerate(data.decode('utf-8', errors='strict').splitlines(), 1):
        clean = ANSI.sub('', raw)
        prefix = PREFIX.match(clean)
        at, body = (prefix['at'], prefix['body']) if prefix else (None, clean)
        if any(word in body for word in ('0b80f250', '7960bc', 'not found', 'failed to load cache')):
            evidence.append({'line': number, 'text': clean})
        match = VERTEX.match(body)
        if not match:
            continue
        label = match['body']
        if match['id'] == '0' and label.startswith('building with '):
            invocation += 1
        key = (invocation, match['id'])
        header = (label.startswith('[') or label.startswith(('importing cache manifest', 'exporting ')))
        if header and not re.match(r'^\d', label):
            if key not in vertices:
                vertices[key] = {'invocation': invocation, 'vertex': match['id'], 'name': label, 'phase': classify(label), 'start_line': number, 'start_log_time': at, 'terminal_updates': []}
        terminal = re.fullmatch(r'DONE ([0-9]+(?:\.[0-9]+)?)s', label)
        cached = label == 'CACHED'
        failed = label.startswith(('ERROR:', 'CANCELED'))
        if key in vertices and (terminal or cached or failed):
            vertices[key]['terminal_updates'].append({'line': number, 'log_time': at, 'state': 'cached' if cached else 'failed' if failed else 'done', 'elapsed_seconds': float(terminal[1]) if terminal else None})
    selected = [v for v in vertices.values() if v['phase'] != 'other']
    for v in selected:
        last = v['terminal_updates'][-1] if v['terminal_updates'] else None
        v['state'] = last['state'] if last else 'incomplete'
        v['elapsed_seconds'] = last['elapsed_seconds'] if last else None
    return {'schema_version': 1, 'log_sha256': sha(path), 'log_bytes': path.stat().st_size, 'uncompressed_log_sha256': hashlib.sha256(data).hexdigest(), 'phases': selected, 'identity_and_failure_lines': evidence,
            'limits': 'BuildKit elapsed is a vertex cost, not job wall time or CPU time. Overlapping vertices are never summed. Cached vertices have no measured execution duration. Combined RUN subphases cannot be recovered retrospectively.'}


def record_phase(args):
    if not args.command:
        raise ValueError('a real phase command is required')
    args.output.mkdir()
    started = datetime.now(timezone.utc).isoformat()
    before = time.monotonic_ns()
    with (args.output / 'stdout').open('xb') as stdout, (args.output / 'stderr').open('xb') as stderr:
        child = subprocess.Popen(args.command, stdout=stdout, stderr=stderr, start_new_session=True)
        def interrupt(_signal, _frame):
            raise KeyboardInterrupt
        previous = {s: signal.signal(s, interrupt) for s in (signal.SIGTERM, signal.SIGINT)}
        try:
            try:
                _, status, usage = os.wait4(child.pid, 0)
            except KeyboardInterrupt:
                os.killpg(child.pid, signal.SIGKILL)
                _, status, usage = os.wait4(child.pid, 0)
            child.returncode = os.waitstatus_to_exitcode(status)
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
    receipt = {'schema_version': 1, 'phase': args.phase, 'command': args.command, 'cwd': os.getcwd(), 'started_utc': started,
               'elapsed_ns': time.monotonic_ns()-before, 'exit_code': child.returncode,
               'user_cpu_seconds': usage.ru_utime, 'system_cpu_seconds': usage.ru_stime, 'child_maxrss_kib_linux': usage.ru_maxrss,
               'stdout_sha256': sha(args.output/'stdout'), 'stderr_sha256': sha(args.output/'stderr'),
               'resource_scope': 'wait4 direct command and its waited descendants; client accounting does not attest an external BuildKit daemon/worker',
               'performance_claim': 'none until paired protocol admission'}
    write(args.output/'phase.json', receipt)
    raise SystemExit(child.returncode if child.returncode >= 0 else 128-child.returncode)


ROLES = ('indra-server', 'gates/server-unit', 'gates/server-native', 'ocr', 'engine-acceptance', 'gates/unit', 'gates/native')


def inventory(root):
    if not root.is_dir() or root.is_symlink():
        raise ValueError('inventory root must be a regular directory')
    records = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('unadmitted inventory entry: ' + str(path))
        records.append({'path': path.relative_to(root).as_posix(), 'type': 'file' if path.is_file() else 'directory',
                        'mode': path.stat().st_mode & 0o7777, 'bytes': path.stat().st_size if path.is_file() else None,
                        'sha256': sha(path) if path.is_file() else None})
    if not records:
        raise ValueError('empty inventory')
    return records


def static_elf(path):
    if not path.is_file() or path.is_symlink() or not os.access(path, os.X_OK):
        raise ValueError('missing regular executable: ' + str(path))
    if path.open('rb').read(4) != b'\x7fELF':
        raise ValueError('not ELF: ' + str(path))
    headers = subprocess.run(['readelf', '-W', '-l', str(path)], capture_output=True, check=True)
    dynamic = subprocess.run(['readelf', '-W', '-d', str(path)], capture_output=True, check=True)
    if re.search(rb'\bINTERP\b', headers.stdout) or re.search(rb'\(NEEDED\)', dynamic.stdout):
        raise ValueError('dynamic ELF dependency/interpreter: ' + str(path))
    return {'sha256': sha(path), 'bytes': path.stat().st_size, 'program_headers': headers.stdout.decode(),
            'dynamic_section': dynamic.stdout.decode(), 'readelf_exit_codes': [headers.returncode, dynamic.returncode]}


def compare_pair(args):
    # The output must be external to every inventoried input.
    for root in (args.baseline, args.candidate, args.baseline_sdk, args.candidate_sdk, args.baseline_materials, args.candidate_materials):
        if args.output.resolve().is_relative_to(root.resolve()):
            raise ValueError('comparison output must be outside all inputs')
    args.output.mkdir()
    receipt = {'schema_version': 1, 'source_protocol_sha256': sha(Path(__file__).with_name('protocol.json')),
               'scope': 'exact binary/static/content gate only; not lifecycle, provenance, SDK durability or speedup admission', 'roles': {}}
    failure = None
    try:
        for role in ROLES:
            baseline = static_elf(args.baseline/role)
            candidate = static_elf(args.candidate/role)
            receipt['roles'][role] = {'baseline': baseline, 'candidate': candidate, 'equal': baseline['sha256'] == candidate['sha256']}
            if baseline['sha256'] != candidate['sha256']:
                raise ValueError('executable identity mismatch: ' + role)
        for label in ('sdk', 'materials'):
            baseline = inventory(getattr(args, 'baseline_'+label))
            candidate = inventory(getattr(args, 'candidate_'+label))
            write(args.output/(label+'-baseline.json'), baseline)
            write(args.output/(label+'-candidate.json'), candidate)
            receipt[label+'_equal'] = baseline == candidate
            if baseline != candidate:
                raise ValueError(label+' inventory differs')
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        failure = str(error)
    receipt.update({'pass': failure is None, 'failure': failure})
    write(args.output/'comparison.json', receipt)
    if failure:
        raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    read = sub.add_parser('read-log')
    read.add_argument('log', type=Path)
    read.add_argument('--output', type=Path, required=True)
    phase = sub.add_parser('record-phase')
    phase.add_argument('--phase', required=True)
    phase.add_argument('--output', type=Path, required=True)
    phase.add_argument('command', nargs=argparse.REMAINDER)
    pair = sub.add_parser('compare-pair')
    for option in ('baseline', 'candidate', 'baseline-sdk', 'candidate-sdk', 'baseline-materials', 'candidate-materials', 'output'):
        pair.add_argument('--'+option, type=Path, required=True)
    args = parser.parse_args()
    if args.action == 'read-log':
        write(args.output, parse_log(args.log))
    elif args.action == 'compare-pair':
        compare_pair(args)
    else:
        if args.command[:1] == ['--']:
            args.command = args.command[1:]
        record_phase(args)


if __name__ == '__main__':
    main()
