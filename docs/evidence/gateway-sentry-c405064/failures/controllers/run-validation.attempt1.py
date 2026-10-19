import hashlib
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parent
IMAGE = 'sha256:56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239'
FIXED = Path('/tmp/indra-gateway-sentry-audit')
BEFORE = Path('/tmp/indra-gateway-sentry-before')
TOOLCHAIN = '/home/christof/.rustup/toolchains/1.99.1-x86_64-unknown-linux-gnu'


def git(source, *args):
    return subprocess.check_output(['git', '-C', str(source), *args], text=True).strip()


sources = {}
for label, source in [('before', BEFORE), ('fixed', FIXED)]:
    if git(source, 'status', '--porcelain'):
        raise RuntimeError('producing source must be clean: ' + label)
    sources[label] = dict(commit=git(source, 'rev-parse', 'HEAD'),
                          tree=git(source, 'rev-parse', 'HEAD^{tree}'),
                          worktree=str(source), clean=True)
(ROOT / 'source-receipts.json').write_text(json.dumps(sources, indent=2) + '\n')
(ROOT / 'validation-script-sha256.json').write_text(json.dumps({
    name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
    for name in ['run-validation.py', 'container-validation.py']}, indent=2) + '\n')
(ROOT / 'preflight-failures.json').write_text(json.dumps([
    dict(command=['docker', 'image', 'ls', '--format', '{{.Repository}}:{{.Tag}} {{.ID}}'],
         status=1, sandbox='default', stderr='permission denied while trying to connect to the docker API at unix:///var/run/docker.sock'),
    dict(command=['sudo', '-n', 'docker', 'image', 'ls', '--format', '{{.Repository}}:{{.Tag}} {{.ID}}'],
         status=1, sandbox='elevated', stderr='sudo: interactive authentication is required'),
    dict(command_description='Initial image inspection included absent optional Entrypoint field',
         status=1, stderr='template parsing error: map has no entry for key Entrypoint'),
    dict(command_description='Bounded network-none preflight of image66e5350 with read-only source',
         status=2, stdout='Pinned Rust1.99.1/Cargo1.99.1 available; only cargo, rustc and rust-std components installed',
         stderr="ls: cannot access '/usr/local/cargo/registry/cache': No such file or directory"),
], indent=2) + '\n')


def stage(mode, source, cpus, affinity):
    name = 'indra-sentry-c405064-' + mode
    command = ['docker', 'create', '--name', name, '--network', 'none',
               '--cpuset-cpus', affinity, '--cpus', str(cpus),
               '--memory', '3g', '--memory-swap', '3g', '--pids-limit', '256',
               '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true',
               '--mount', 'type=bind,src=' + str(source) + ',dst=/app,readonly',
               '--mount', 'type=bind,src=' + TOOLCHAIN + ',dst=/host-toolchain,readonly',
               '--mount', 'type=bind,src=' + str(ROOT) + ',dst=/evidence',
               '--mount', 'type=volume,src=indra-issue13-cargo-registry,dst=/usr/local/cargo/registry',
               '--mount', 'type=volume,src=indra-issue13-cargo-target,dst=/target',
               '--env', 'PATH=/host-toolchain/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
               '--env', 'CARGO_BUILD_JOBS=2', '--env', 'CARGO_INCREMENTAL=0',
               '--env', 'CARGO_TARGET_DIR=/target/gateway-sentry-c405064',
               '--env', 'CARGO_NET_OFFLINE=true', '--env', 'RUSTFLAGS=-Ctarget-feature=+crt-static',
               '--entrypoint', 'python3', IMAGE, '/evidence/container-validation.py', mode]
    receipt = dict(mode=mode, source=sources['before' if source == BEFORE else 'fixed'],
                   create_command=command, started_unix=time.time())
    (ROOT / (mode + '-container-command.json')).write_text(json.dumps(receipt, indent=2) + '\n')
    created = subprocess.run(command, text=True, capture_output=True)
    (ROOT / (mode + '-create.stdout')).write_text(created.stdout)
    (ROOT / (mode + '-create.stderr')).write_text(created.stderr)
    if created.returncode:
        raise RuntimeError('container creation failed: ' + mode)
    identifier = created.stdout.strip()
    receipt['container_id'] = identifier
    with (ROOT / (mode + '-inspect-before.json')).open('wb') as stream:
        subprocess.run(['docker', 'inspect', identifier], stdout=stream, check=True)
    print('START ' + mode + ' ' + identifier, flush=True)
    with (ROOT / (mode + '-container.stdout')).open('wb') as out, (ROOT / (mode + '-container.stderr')).open('wb') as err:
        result = subprocess.run(['docker', 'start', '-a', identifier], stdout=out, stderr=err)
    with (ROOT / (mode + '-inspect-after.json')).open('wb') as stream:
        subprocess.run(['docker', 'inspect', identifier], stdout=stream, check=True)
    receipt.update(status=result.returncode, finished_unix=time.time())
    (ROOT / (mode + '-container-command.json')).write_text(json.dumps(receipt, indent=2) + '\n')
    print('FINISH ' + mode + ' status=' + str(result.returncode), flush=True)
    if result.returncode:
        print((ROOT / (mode + '-container.stderr')).read_text(), flush=True)
        raise RuntimeError('validation container failed: ' + mode)


stage('before-fix', BEFORE, 2, '12,13')
stage('fixed-checks', FIXED, 2, '12,13')
stage('process-default-1cpu', FIXED, 1, '12')
stage('process-default-2cpu', FIXED, 2, '12,13')
stage('process-sentry-only', FIXED, 2, '12,13')
(ROOT / 'completed.json').write_text(json.dumps(dict(passed=True, sources=sources, image=IMAGE,
    scope='Unmeasured correctness only; no native OCR, no native C++ rebuild, no DOCS_RS; Docker process tests at1/2CPU and3GiB/zeroSwap'), indent=2) + '\n')
print('ALL VALIDATION STAGES PASSED', flush=True)
