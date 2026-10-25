#!/usr/bin/env python3
"""Independently admit retained native ablation records; never execute OCR."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import shlex
import statistics
import sys

MEMORY = 3 * 1024**3
IMAGE = 'sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142'
SOURCE = 'ad048cefe35d950e82806fd56a65a91b84033848'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return path.read_text(encoding='utf-8')


def load(path):
    return json.loads(read(path))


def json_stream(path):
    # The controller's processes.jsonl is a stream of pretty-printed JSON objects.
    text = read(path)
    decoder = json.JSONDecoder()
    result = []
    offset = 0
    while offset < len(text):
        while offset < len(text) and text[offset].isspace():
            offset += 1
        if offset == len(text):
            break
        value, offset = decoder.raw_decode(text, offset)
        result.append(value)
    return result


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def counters(path):
    rows = [row.split() for row in read(path).splitlines()]
    require(all(len(row) == 2 for row in rows), f'invalid counters: {path}')
    result = {key: int(value) for key, value in rows}
    require(len(result) == len(rows), f'duplicate counter: {path}')
    return result


def finite(value, positive=False, integer=False):
    return (type(value) in (int, float) and math.isfinite(value)
            and (value > 0 if positive else value >= 0)
            and (not integer or value == int(value)))


def manifest(root, path):
    names = set()
    for row in read(path).splitlines():
        digest, name = row.split('  ', 1)
        relative = Path(name)
        require(re.fullmatch('[0-9a-f]{64}', digest) is not None
                and not relative.is_absolute() and '..' not in relative.parts
                and name not in names, f'invalid manifest entry: {row}')
        names.add(name)
        target = root / relative
        require(target.is_file() and not target.is_symlink() and sha(target) == digest,
                f'manifest mismatch: {target}')
    require(names, f'empty manifest: {path}')
    return len(names)


def original_materials(thin, fat, models, model_lock):
    result = {}
    for lto, bundle in [('thin', thin), ('fat', fat)]:
        result[lto] = {'artifact_manifest_entries': manifest(bundle, bundle / 'artifacts.sha256'),
                       'relink_manifest_entries': manifest(bundle / 'relink', bundle / 'relink/SHA256SUMS')}
        source = dict(row.split('=', 1) for row in read(bundle / 'build-evidence/execution-source.txt').splitlines())
        require(source == {'source_commit': SOURCE, 'cargo_profile': 'profiling', 'lto': lto}, 'build source/profile mismatch')
        for stem in ['ocr-profiling', 'gates-unit', 'gates-native', 'gates-perf-control', 'gates-ocr-cli',
                     'gates-allocations', 'gates-core-allocations', 'gates-telemetry-allocations', 'gates-model-allocations']:
            require('INTERP' not in read(bundle / f'build-evidence/{stem.removeprefix("gates-")}.program-headers.txt'),
                    'dynamic interpreter')
            require('(NEEDED)' not in read(bundle / f'build-evidence/{stem.removeprefix("gates-")}.dynamic.txt'),
                    'dynamic dependency')
        result[lto]['executed_elf_sha256'] = sha(bundle / 'ocr-profiling')
        for name in ['relink/rust-source.tar.gz', 'relink/native-link-inputs.tar.gz', 'build-evidence/rustc.txt',
                     'build-evidence/static-archives.sha256', 'native-ablation-protocol.json']:
            result[lto][name] = sha(bundle / name)
    for name in ['relink/rust-source.tar.gz', 'relink/native-link-inputs.tar.gz', 'build-evidence/rustc.txt',
                 'build-evidence/static-archives.sha256', 'native-ablation-protocol.json']:
        require(result['thin'][name] == result['fat'][name], f'paired materials differ: {name}')
    require(result['thin']['executed_elf_sha256'] != result['fat']['executed_elf_sha256'], 'thin/fat ELF unexpectedly identical')
    lock = load(model_lock)
    checked = []
    for entry in lock['files']:
        path = models / entry['name']
        require(path.is_file() and path.stat().st_size == entry['size'] and sha(path) == entry['sha256'],
                f'model mismatch: {path}')
        checked.append({'name': entry['name'], 'size': entry['size'], 'sha256': entry['sha256']})
    result['models'] = {'lock_sha256': sha(model_lock), 'revision': lock['revision'], 'files': checked}
    result['large_material_bytes_verified'] = True
    return result


def phase_admission(run, name, cpus, kind, row=None):
    outer = run / name
    phase = outer / f'cpu-{cpus}' / kind
    status = load(outer / 'status.json')
    require(status['exit'] == status['cleanup_exit'] == 0, f'outer failure: {name}')
    require(load(outer / 'completion.json')['completed'] is True, f'incomplete: {name}')
    require(load(phase / 'phase-status.json') == {'exit': 0, 'resource_capture_exit': 0}, f'phase failure: {name}')
    before = load(phase / 'container-before.json')[0]
    running = load(phase / 'container-running.json')[0]
    final = load(phase / 'container.json')[0]
    admission = load(phase / 'host-admission.json')
    cid = before['Id']
    owner = before['Config']['Labels']['org.tika-indra.native-profile']
    require(re.fullmatch('[0-9a-f]{64}', cid) is not None and re.fullmatch('[0-9a-f]{32}', owner) is not None,
            f'invalid identity: {name}')
    require(all(x['Id'] == cid and x['Image'] == IMAGE
                and x['Config']['Labels']['org.tika-indra.native-profile'] == owner
                for x in [before, running, final]), f'container/image/owner mismatch: {name}')
    require(admission['admitted'] is True and admission['container_id'] == cid
            and admission['host_pid'] == running['State']['Pid'] > 0 and running['State']['Running'] is True
            and admission['cgroup'].endswith(f'/docker-{cid}.scope')
            and read(phase / 'host-process.cgroup').strip().endswith(f'/docker-{cid}.scope'), f'host PID/cgroup mismatch: {name}')
    affinity = '0' if cpus == 1 else '0,1'
    effective = '0' if cpus == 1 else '0-1'
    h = running['HostConfig']
    for key, expected in {'NanoCpus': cpus * 10**9, 'CpusetCpus': affinity, 'Memory': MEMORY,
                          'MemorySwap': MEMORY, 'PidsLimit': 128, 'CgroupnsMode': 'private',
                          'NetworkMode': 'none', 'ReadonlyRootfs': True, 'Privileged': False,
                          'PidMode': '', 'CapDrop': ['ALL'], 'SecurityOpt': ['no-new-privileges']}.items():
        require(h.get(key) == expected, f'Docker resource mismatch {key}: {name}')
    require(not h.get('CapAdd'), f'extra capability: {name}')
    require({x['Name']: (x['Soft'], x['Hard']) for x in h['Ulimits']} ==
            {'stack': (8388608, 8388608), 'memlock': (1140850688, 1140850688)}, f'limits mismatch: {name}')
    mounts = {x['Destination']: x for x in running['Mounts']}
    require(len(mounts) == 6 and set(mounts) == {'/bundle', '/models', '/evidence', '/work/docker/profile-tools.Dockerfile',
                                                               '/work/scripts/profile-native.sh', '/work/scripts/verify-native-profile-samples.sh'}
            and all(x['RW'] == (dest == '/evidence') for dest, x in mounts.items())
            and mounts['/bundle']['Type'] == 'volume' and mounts['/bundle']['Driver'] == 'local', f'mount mismatch: {name}')
    volume = load(outer / 'cleanup-volume.json')[0]
    require(volume['Name'] == mounts['/bundle']['Name'] and volume['Driver'] == 'local' and volume['Options'] is None
            and volume['Labels']['org.tika-indra.native-profile'] == owner, f'volume identity mismatch: {name}')
    require(read(outer / 'cleanup-volumes-remaining.txt') == '' and read(outer / 'cleanup-volume-rm.txt').strip() == volume['Name'],
            f'volume removal mismatch: {name}')
    require(read(outer / 'host-bundle-owner.before') == read(outer / 'host-bundle-owner.after'), f'host permissions changed: {name}')
    seed = load(outer / 'seed-final.json')[0]
    require(seed['State']['ExitCode'] == 0 and seed['State']['OOMKilled'] is False and seed['Image'] == IMAGE,
            f'private bundle seed failed: {name}')
    require(read(outer / 'host-bundle.inventory') == read(outer / 'volume-bundle.inventory'), f'private bundle inventory mismatch: {name}')
    for boundary in ['before', 'after']:
        snapshot = phase / boundary
        for key, expected in {'cpu.max': f'{cpus*100000} 100000', 'memory.max': str(MEMORY), 'memory.swap.max': '0',
                              'pids.max': '128', 'cpuset.cpus.effective': effective, 'stack-soft-kib': '8192'}.items():
            require(read(snapshot / key).strip() == expected, f'actual resource mismatch {key}: {name}')
        require(re.search(r'^Cpus_allowed_list:\s+' + re.escape(effective) + r'$', read(snapshot / 'process.status'), re.M),
                f'actual process affinity mismatch: {name}')
        require(re.search(r'^Max stack size\s+8388608\s+8388608\s+bytes\s*$', read(snapshot / 'process.limits'), re.M),
                f'actual stack mismatch: {name}')
        require(counters(snapshot / 'memory.events')['oom_kill'] == 0, f'OOM event: {name}')
    ancestors = read(phase / 'host-ancestors.txt').splitlines()
    require(ancestors[0] == admission['cgroup'] and ancestors[-1] == '/sys/fs/cgroup', f'ancestor traversal mismatch: {name}')
    for entry in ancestors:
        if entry.startswith('cpu.max '):
            quota, period = entry.removeprefix('cpu.max ').split()
            require(quota == 'max' or int(quota) / int(period) >= cpus, f'tighter CPU ancestor: {name}')
        elif entry.startswith('memory.max '):
            limit = entry.removeprefix('memory.max ')
            require(limit == 'max' or int(limit) >= MEMORY, f'tighter memory ancestor: {name}')
    require(final['State']['Running'] is False and final['State']['OOMKilled'] is False and final['State']['ExitCode'] == 0
            and read(phase / 'removed.txt').strip() == cid, f'exit/removal mismatch: {name}')
    if row:
        arguments = running['Args']
        require(arguments[0:3] == ['--signal=TERM', '--kill-after=15s', '900s']
                and arguments[5:] == ['--ablation-phase', '/bundle', '/models', str(cpus), affinity, row['fixture'],
                                      str(row['settings']['batch_size']), str(row['settings']['spinning']).lower(), '/evidence'],
                f'outer phase arguments mismatch: {name}')
    return phase, {'id': name, 'cpus': cpus, 'container_id': cid, 'host_pid': admission['host_pid'],
                   'owner_token': owner, 'image_id': IMAGE, 'cgroup': admission['cgroup'], 'admitted': True}


def process_summary(run, row):
    phase, identity = phase_admission(run, row['id'], row['cpus'], 'ablation', row)
    requests = [json.loads(line) for line in read(phase / 'requests.jsonl').splitlines()]
    require(len(requests) == 32 and [x['iteration'] for x in requests] == list(range(32))
            and len({x['process_id'] for x in requests}) == 1, f'request identities/count: {row["id"]}')
    math_fixture = row['fixture'] == 'math'
    for request in requests:
        exact = {'diagnostic': False, 'threads': row['cpus'], 'cv_threads': row['cpus'], 'math': math_fixture,
                 'batch_size': row['settings']['batch_size'], 'intra_op_spinning': row['settings']['spinning'],
                 'inter_op_spinning': row['settings']['spinning'], 'frames': 1 if math_fixture else 2,
                 'output_bytes': 293 if math_fixture else 33}
        require(all(type(request[k]) is type(value) and request[k] == value for k, value in exact.items()),
                f'request configuration mismatch: {row["id"]}')
        require(finite(request['process_id'], positive=True, integer=True) and request['process_id'] <= 2**32-1
                and finite(request['elapsed_us'], positive=True)
                and len(request['stage_us']) == 7 and all(finite(x) for x in request['stage_us'])
                and len(request['batch_counts']) > 0 and all(finite(x, integer=True) for x in request['batch_counts'])
                and finite(request['valid_columns'], integer=True) and finite(request['padded_columns'], integer=True)
                and request['padded_columns'] >= request['valid_columns'], f'invalid request journal: {row["id"]}')
    command = shlex.split(read(phase / 'recognition.command.txt'))
    expected = ['--threads', str(row['cpus']), '--cv-threads', str(row['cpus']), '--batch-size', str(row['settings']['batch_size']),
                '--intra-op-spinning', str(row['settings']['spinning']).lower(), '--inter-op-spinning', str(row['settings']['spinning']).lower(),
                '--warmup', '--iterations', '32', '--timeout-seconds', '60', '--expect',
                '/bundle/fixtures/math-page.expected.txt' if math_fixture else '/bundle/fixtures/two-pages.expected.txt', '--expect-each',
                '--output', '/evidence/output.txt', '--stats', '/evidence/requests.jsonl'] + (['--math'] if math_fixture else [])
    require(command[0] == '/bundle/ocr-profiling' and command[1] == ('/bundle/fixtures/math.png' if math_fixture else '/bundle/fixtures/two-pages.tiff') and command[2:4] == ['--model-dir', '/models'] and command[4:] == expected
            and read(phase / 'recognition.exit').strip() == '0' and read(phase / 'request-admission.txt').strip() == 'true',
            f'recognition command/parity gate: {row["id"]}')
    fixture = 'math-page.expected.txt' if math_fixture else 'two-pages.expected.txt'
    expected_digest = dict(line.split('  ', 1)[::-1] for line in read(run / row['id'] / 'fixture-sha256.txt').splitlines())[fixture]
    require(sha(phase / 'output.txt') == expected_digest, f'accepted UTF-8 mismatch: {row["id"]}')
    x = sorted(request['elapsed_us'] for request in requests)
    before = counters(phase / 'before/cpu.stat')
    after = counters(phase / 'after/cpu.stat')
    peak = int(read(phase / 'after/memory.peak'))
    require(0 < peak <= MEMORY and after['usage_usec'] >= before['usage_usec']
            and after['throttled_usec'] >= before['throttled_usec'], f'resource counter regression: {row["id"]}')
    summary = dict(row, p50_us=x[15], p95_us=x[30], p99_us=x[31], warmed_recognitions_per_second=32000000 / sum(x),
                   phase_cpu_usage_us=after['usage_usec']-before['usage_usec'],
                   phase_throttled_us=after['throttled_usec']-before['throttled_usec'], phase_peak_memory_bytes=peak,
                   request_count=32, valid_columns=sum(r['valid_columns'] for r in requests),
                   padded_columns=sum(r['padded_columns'] for r in requests))
    return summary, identity


def admit(run):
    require(load(run / 'status.json') == {'exit': 0, 'interrupted_controller_exit': 0}, 'master failure')
    require(load(run / 'completion.json') == {'completed': True, 'processes': 128, 'requests': 4096,
            'scope': 'native synthetic fixture one-factor ablations; not total Tika'}, 'incomplete study')
    require(sha(run / 'protocol.json') == '15dcecb375f1ed06c5fbb04811ec51e4b126a8b260965cbe0680e84151d1e6f7', 'frozen protocol identity mismatch')
    protocol = load(run / 'protocol.json')
    expected = []
    for cpus in protocol['cpus']:
        for fixture in protocol['fixtures']:
            for comparison in protocol['comparisons']:
                for block, order in enumerate(protocol['paired_block_order']):
                    for arm in order:
                        expected.append({'cpus': cpus, 'fixture': fixture, 'comparison': comparison['id'], 'block': block,
                                         'order': order, 'arm': arm,
                                         'settings': protocol['baseline'] if arm == 'A' else {k: v for k, v in comparison.items() if k != 'id'},
                                         'id': f'cell-{len(expected)+1}'})
    require(len(expected) == 128 and load(run / 'schedule.json') == expected, 'prospective schedule mismatch')
    identities = []
    for lto in ['thin', 'fat']:
        for cpus in [1, 2]:
            phase, identity = phase_admission(run, f'gates-{lto}-{cpus}', cpus, 'gates')
            for gate in ['unit', 'native', 'perf-control', 'ocr-cli', 'allocations', 'core-allocations', 'telemetry-allocations', 'model-allocations']:
                require(read(phase / f'{gate}.exit').strip() == '0', f'failed native gate {lto}/{cpus}/{gate}')
                suffix = {'unit': ['--include-ignored', '--test-threads=1', '--nocapture'],
                          'native': ['--ignored', '--test-threads=1', '--nocapture'],
                          'perf-control': ['--test-threads=1', '--nocapture'],
                          'ocr-cli': ['--test-threads=1', '--nocapture']}.get(gate, [])
                require(shlex.split(read(phase / f'{gate}.command.txt')) == [f'/bundle/gates/{gate}'] + suffix,
                        f'native gate command mismatch {lto}/{cpus}/{gate}')
            require('35 passed; 0 failed' in read(phase / 'unit.stdout.log') and '3 passed; 0 failed' in read(phase / 'native.stdout.log'),
                    f'missing native test success: {lto}/{cpus}')
            identities.append(identity)
    summaries = []
    for row in expected:
        summary, identity = process_summary(run, row)
        summaries.append(summary)
        identities.append(identity)
    require(summaries == json_stream(run / 'processes.jsonl'), 'independent process arithmetic mismatch')
    pairs = []
    for i in range(0, 128, 2):
        by_arm = {x['arm']: x for x in summaries[i:i+2]}
        a, b = by_arm['A'], by_arm['B']
        pairs.append({'cpus': a['cpus'], 'fixture': a['fixture'], 'comparison': a['comparison'], 'block': a['block'], 'order': a['order'],
                      'baseline': a, 'candidate': b, 'median_ratio': b['p50_us'] / a['p50_us'],
                      'p95_ratio': b['p95_us'] / a['p95_us'], 'peak_memory_ratio': b['phase_peak_memory_bytes'] / a['phase_peak_memory_bytes']})
    recorded = load(run / 'analysis.json')
    key = lambda x: (x['cpus'], x['fixture'], x['comparison'], x['block'])
    require(sorted(pairs, key=key) == recorded['pairs'], 'independent paired arithmetic mismatch')
    groups = []
    for cpus in [1, 2]:
        for fixture in ['tiff', 'math']:
            for comparison in protocol['comparisons']:
                selected = [p for p in pairs if p['cpus'] == cpus and p['fixture'] == fixture and p['comparison'] == comparison['id']]
                group = {'cpus': cpus, 'fixture': fixture, 'comparison': comparison['id'], 'process_pairs': 4,
                         'slower_candidate_medians': sum(p['median_ratio'] > 1 for p in selected)}
                for field in ['median_ratio', 'p95_ratio', 'peak_memory_ratio']:
                    values = [p[field] for p in selected]
                    group[field] = {'min': min(values), 'median': statistics.median(values), 'max': max(values)}
                groups.append(group)
    require(len({x['container_id'] for x in identities}) == 132, 'container reused across fresh processes/gates')
    decisions = []
    for comparison in sorted(x['id'] for x in protocol['comparisons']):
        passed = all(p['median_ratio'] <= 0.95 and p['p95_ratio'] <= 1.05 and p['peak_memory_ratio'] <= 1.10
                     for p in pairs if p['comparison'] == comparison)
        decisions.append({'comparison': comparison, 'decision':
            'candidate meets conservative synthetic-fixture policy; confirm on representative workload' if passed
            else 'retain baseline; candidate fails conservative synthetic-fixture policy'})
    require(decisions == recorded['decisions'], 'independent prospective decision mismatch')
    return {'admitted': True, 'native_gate_roles': 4, 'native_gate_commands': 32, 'measured_processes': 128,
            'measured_requests': 4096, 'process_pairs': 64, 'identities': identities, 'groups': groups,
            'decisions': recorded['decisions'], 'scope': 'Four descriptive process pairs per CPU/fixture/comparison; nearest-rank within-process tails are not population p95/p99 estimates.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('records', type=Path)
    parser.add_argument('--thin', type=Path)
    parser.add_argument('--fat', type=Path)
    parser.add_argument('--models', type=Path)
    parser.add_argument('--model-lock', type=Path)
    args = parser.parse_args()
    try:
        result = admit(args.records)
        materials = [args.thin, args.fat, args.models, args.model_lock]
        require(all(materials) or not any(materials), 'supply all four original-material arguments together')
        if all(materials):
            result['original_materials'] = original_materials(*materials)
        print(json.dumps(result, indent=2, sort_keys=True))
    except (ValueError, KeyError, OSError, TypeError, IndexError) as error:
        print(f'ablation evidence rejected: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
