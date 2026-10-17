"""Run correctness smoke checks after bounding all app threads by CPU affinity."""
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys

base = Path('/workspace/scratch/b641098e5d96')
repository = base / 'tika-indra-ocr-client'
output = repository / 'validation/rootfs'
output.mkdir(parents=True, exist_ok=True)
roots = {str(base / 'docker-validation' / name): name for name in ('java-root', 'python-root', 'tesseract-root')}

def apply_affinity(cpus):
    observations = []
    for process in Path('/proc').iterdir():
        if not process.name.isdigit():
            continue
        try:
            root = os.readlink(process / 'root')
            if root not in roots:
                continue
            tids = list((process / 'task').iterdir())
            for thread in tids:
                os.sched_setaffinity(int(thread.name), cpus)
            masks = {tuple(sorted(os.sched_getaffinity(int(thread.name)))) for thread in tids}
            if masks != {tuple(sorted(cpus))}:
                raise RuntimeError(f'Unexpected task affinities {masks}')
            mounts = {}
            for line in (process / 'mountinfo').read_text().splitlines():
                fields = line.split()
                if fields[4] in ('/', '/tmp'):
                    split = fields.index('-')
                    mounts[fields[4]] = {'options': fields[5].split(','), 'filesystem': fields[split + 1],
                                         'superblock_options': fields[split + 3].split(',') if fields[split + 1] == 'tmpfs' else []}
            if 'ro' not in mounts['/']['options']:
                raise RuntimeError('Application root is writable')
            if not {'noexec', 'nosuid'} <= set(mounts['/tmp']['options']):
                raise RuntimeError('Missing tmpfs protections')
            uid_line = next(line for line in (process / 'status').read_text().splitlines() if line.startswith('Uid:'))
            uids = [int(value) for value in uid_line.split()[1:]]
            if set(uids) != {65532}:
                raise RuntimeError(f'Unexpected application UID values {uids}')
            observations.append({'pid': int(process.name), 'root': roots[root], 'threads': len(tids),
                                 'every_thread_affinity': sorted(cpus), 'uid': 65532, 'mounts': mounts})
        except FileNotFoundError:
            continue
    if {row['root'] for row in observations} != set(roots.values()):
        raise RuntimeError('An expected application root has no live process')
    return observations

for cpus in ({4}, {4, 5}):
    observations = apply_affinity(cpus)
    report = {'checked_at_utc': datetime.now(timezone.utc).isoformat(),
              'scope': 'nonroot read-only exported image filesystems in private namespaces',
              'cpu_constraint': 'affinity of every application thread; no CFS quotas',
              'cpu_affinity': sorted(cpus), 'docker_build': False, 'docker_run': False,
              'compose_executed': False, 'kubernetes_executed': False,
              'runtime_observations': observations, 'stacks': {}}
    for label, port in [('tika_indra', 19998), ('tika_tesseract_best', 19997)]:
        command = [sys.executable, str(repository / 'scripts/smoke_stack.py'),
                   '--url', f'http://127.0.0.1:{port}', '--wait', '20']
        result = subprocess.run(command, check=False, capture_output=True, text=True, timeout=180)
        if result.returncode:
            (output / f'{label}-{len(cpus)}cpu-failure.log').write_text(result.stdout + result.stderr)
            raise RuntimeError(f'{label} failed at {len(cpus)} CPU affinity: {result.stderr}')
        report['stacks'][label] = json.loads(result.stdout)
        print(f'{label}: PNG, scanned PDF, native PDF and recursive metadata passed at {len(cpus)} CPU affinity', flush=True)
    report['runtime_observations_after'] = apply_affinity(cpus)
    (output / f'stack-{len(cpus)}cpu.json').write_text(json.dumps(report, indent=2) + '\n')
