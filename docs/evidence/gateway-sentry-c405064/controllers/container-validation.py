import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path('/evidence')
MODE = sys.argv[1]
ATTEMPT = os.environ.get('VALIDATION_ATTEMPT', '')
OUT = ROOT / (MODE + ('-' + ATTEMPT if ATTEMPT else ''))
OUT.mkdir()
TARGET = 'x86_64-unknown-linux-gnu'
BASE = ['--offline', '--locked', '--release', '--manifest-path', '/app/runtime/Cargo.toml', '--target', TARGET]


def run(name, args, failure=False, cwd='/app'):
    begin = time.time()
    with (OUT / (name + '.stdout')).open('wb') as out, (OUT / (name + '.stderr')).open('wb') as err:
        result = subprocess.run(args, cwd=cwd, stdout=out, stderr=err)
    record = dict(name=name, command=args, cwd=cwd, status=result.returncode,
                  expected_failure=failure, start_unix=begin, finish_unix=time.time(),
                  target_dir=os.environ.get('CARGO_TARGET_DIR'), rustflags=os.environ.get('RUSTFLAGS'))
    with (OUT / 'commands.jsonl').open('a') as stream:
        stream.write(json.dumps(record) + '\n')
    print(json.dumps(record), flush=True)
    if failure:
        if result.returncode == 0:
            raise RuntimeError('before-fix correlation regression unexpectedly passed')
        text = (OUT / (name + '.stderr')).read_text()
        if 'Null' not in text or '0000000000000001' not in text:
            raise RuntimeError('before-fix regression failed without the expected dropped correlation evidence')
    elif result.returncode:
        raise RuntimeError(name + ' failed')
    return OUT / (name + '.stdout')


def select(json_path, prefix):
    records = [json.loads(line) for line in json_path.read_text().splitlines() if line.startswith('{')]
    chosen = {}
    for record in records:
        if record.get('reason') != 'compiler-artifact' or not record.get('executable'):
            continue
        target = record.get('target', {})
        if target.get('name') == 'process_lifecycle' and target.get('kind') == ['test']:
            role = 'harness'
        elif target.get('name') == 'indra-gateway' and target.get('kind') == ['bin'] and not record.get('profile', {}).get('test'):
            role = 'gateway'
        else:
            continue
        if role in chosen:
            raise RuntimeError('ambiguous produced ' + role)
        source = Path(record['executable'])
        destination = ROOT / 'bin' / (prefix + '-' + role)
        destination.parent.mkdir(exist_ok=True)
        shutil.copy2(source, destination)
        chosen[role] = dict(source=str(source), artifact=str(destination),
                            sha256=hashlib.sha256(destination.read_bytes()).hexdigest(),
                            cargo_artifact=record)
    if set(chosen) != {'harness', 'gateway'}:
        raise RuntimeError('missing exact Cargo harness/gateway artifact')
    local = [record for record in records if record.get('reason') == 'compiler-artifact'
             and any('/' + package + '#' in record.get('package_id', '')
                     for package in ['indra-telemetry', 'indra-gateway'])]
    if any(record.get('fresh') is not False for record in local):
        raise RuntimeError('affected package was not freshly rebuilt after source/variant transition')
    if not any('/indra-telemetry#' in record.get('package_id', '') for record in local):
        raise RuntimeError('fresh reporter producing artifact absent')
    (OUT / (prefix + '-fresh-local-artifacts.json')).write_text(json.dumps(local, indent=2) + '\n')
    (OUT / (prefix + '-selection.json')).write_text(json.dumps(chosen, indent=2) + '\n')


def process(prefix, targeted=False, expected_failure=False):
    os.environ['INDRA_TEST_GATEWAY_BIN'] = str(ROOT / 'bin' / (prefix + '-gateway'))
    command = [str(ROOT / 'bin' / (prefix + '-harness'))]
    if targeted:
        command.append('--sentry-correlation-only')
    run('process', command, failure=expected_failure)


limits = {}
for name in ['cpu.max', 'cpuset.cpus.effective', 'memory.max', 'memory.swap.max', 'pids.max', 'cpu.stat', 'memory.events']:
    path = Path('/sys/fs/cgroup') / name
    if path.exists():
        limits[name] = path.read_text()
(OUT / 'cgroup-before.json').write_text(json.dumps(limits, indent=2) + '\n')
run('rustc', ['rustc', '-Vv'])
run('cargo', ['cargo', '-V'])

if MODE == 'before-fix':
    log = run('build-sentry-only', ['cargo', 'test'] + BASE + ['-p', 'indra-gateway', '--no-default-features', '--features', 'sentry', '--test', 'process_lifecycle', '--no-run', '--message-format=json-render-diagnostics'])
    select(log, 'before')
    process('before', targeted=True, expected_failure=True)
elif MODE == 'fixed-checks':
    all_target = '/target/gateway-sentry-c405064/c405064-attempt3-all-features'
    sentry_target = '/target/gateway-sentry-c405064/c405064-attempt3-sentry-only'
    for directory in [all_target, sentry_target]:
        Path(directory).mkdir(exist_ok=False)
    os.environ['CARGO_TARGET_DIR'] = all_target
    run('clean-owned-crates', ['cargo', 'clean', '--release', '--manifest-path', '/app/runtime/Cargo.toml', '--target', TARGET, '-p', 'indra-telemetry', '-p', 'indra-gateway'])
    run('reporter-unit-all-features', ['cargo', 'test'] + BASE + ['-p', 'indra-telemetry', '--all-features', '--lib'])
    run('gateway-unit-all-features', ['cargo', 'test'] + BASE + ['-p', 'indra-gateway', '--all-features', '--lib'])
    run('clean-before-default-producer', ['cargo', 'clean', '--release', '--manifest-path', '/app/runtime/Cargo.toml', '--target', TARGET, '-p', 'indra-telemetry', '-p', 'indra-gateway'])
    log = run('build-default', ['cargo', 'test'] + BASE + ['-p', 'indra-gateway', '--test', 'process_lifecycle', '--no-run', '--message-format=json-render-diagnostics'])
    select(log, 'default-c405064-a3')
    os.environ['CARGO_TARGET_DIR'] = sentry_target
    log = run('build-sentry-only', ['cargo', 'test'] + BASE + ['-p', 'indra-gateway', '--no-default-features', '--features', 'sentry', '--test', 'process_lifecycle', '--no-run', '--message-format=json-render-diagnostics'])
    select(log, 'sentry-only-c405064-a3')
    os.environ['CARGO_TARGET_DIR'] = all_target
    run('clippy', ['cargo', 'clippy'] + BASE + ['-p', 'indra-gateway', '-p', 'indra-telemetry', '--all-features', '--all-targets', '--', '-D', 'warnings'])
    run('fmt', ['cargo', 'fmt', '--manifest-path', '/app/runtime/Cargo.toml', '--all', '--', '--check'])
    receipts = {}
    for path in sorted((ROOT / 'bin').iterdir()):
        programs = run(path.name + '-program-headers', ['readelf', '-W', '-l', str(path)])
        dynamic = run(path.name + '-dynamic', ['readelf', '-W', '-d', str(path)])
        header = run(path.name + '-elf-header', ['readelf', '-W', '-h', str(path)])
        if 'INTERP' in programs.read_text() or 'NEEDED' in dynamic.read_text():
            raise RuntimeError('ELF contains a dynamic interpreter/dependency: ' + path.name)
        if 'UNIX - GNU' not in header.read_text() and 'UNIX - System V' not in header.read_text():
            raise RuntimeError('ELF OS/ABI unexpected')
        receipts[path.name] = dict(sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                                   target=TARGET, rustflags=os.environ['RUSTFLAGS'],
                                   interpreter=False, dynamic_dependencies=False)
    (OUT / 'elf-receipts.json').write_text(json.dumps(receipts, indent=2) + '\n')
elif MODE.startswith('process-default-'):
    process('default-c405064-a3')
elif MODE == 'process-sentry-only':
    process('sentry-only-c405064-a3', targeted=True)
else:
    raise RuntimeError('unknown mode')

after = {}
for name in ['cpu.stat', 'memory.events', 'memory.peak', 'pids.peak']:
    path = Path('/sys/fs/cgroup') / name
    if path.exists():
        after[name] = path.read_text()
(OUT / 'cgroup-after.json').write_text(json.dumps(after, indent=2) + '\n')
(OUT / 'passed.json').write_text(json.dumps(dict(mode=MODE, passed=True)) + '\n')
