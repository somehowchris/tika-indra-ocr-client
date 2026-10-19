#!/usr/bin/env python3
"""Replay the frozen continuous pilot records and descriptive engineering screen.

Analysis only: this script never executes OCR or changes a retained study.
"""
import argparse
import calendar
from datetime import datetime
from fractions import Fraction
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import re
import statistics
import sys

SOURCE = '85fef713f22dd136e2844cb00e6761b299c057cb'
PROTOCOL = 'ec95074659ae6bf7d92412c1a1e073e0c9a89f590424559e12a9825ec87aad2a'
MODELS = 'fc1f2ba8c79c500f9f513ea7b3bc1d5133681d9882450cbd5e9ec00b57ed8f7d'
IMAGE = 'sha256:cbc65a508d432318d98a53587f555bb60f54737327918ec49b3501f5c6feda92'
MEMORY = 3 * 1024**3
SCHEDULE = ['smoke', 'two-pages', 'math-page', 'smoke', 'math-page', 'two-pages',
            'two-pages', 'smoke', 'math-page', 'math-page', 'two-pages', 'smoke']
NATIVE_RECEIPT = '7e430b83a41d3f4b18963690d9b3be27e38c240ecf6a74b96609f5bf275833dd'
IDENTITIES = {'native': '2d1e2526aa494e12f6de5fb39ea3771cbf2c6b357c3a1f25d161973025178041',
              'tika': 'c7b2f7e5dc7eb3fdd05c42617dde50bb0671d447b65485cfb7b7e76e703d10a9'}
DRIVERS = {'native': 'd670c063442f75ffdafdb4c13b1b6cd8da408981f08a6653323489443c119a85',
           'tika': 'b84c7bf97e169049b4f25d18e6dc4d363c1ad42185c842f1db74c4bd49145ca4'}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def load(path):
    return json.loads(path.read_text(encoding='utf-8'))


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def number(value, positive=False):
    return type(value) is int and (value > 0 if positive else value >= 0)


def counters(path):
    result = {}
    for line in path.read_text().splitlines():
        parts = line.split()
        require(len(parts) == 2 and parts[0] not in result and parts[1].isdigit(),
                f'invalid or duplicate counter: {path}')
        result[parts[0]] = int(parts[1])
    return result


def manifest(root, layer):
    names = set()
    for line in (root / 'SHA256SUMS').read_text().splitlines():
        digest, name = line.split('  ', 1)
        relative = Path(name)
        require(re.fullmatch('[0-9a-f]{64}', digest) is not None
                and not relative.is_absolute() and '..' not in relative.parts
                and name not in names, 'invalid raw manifest entry')
        names.add(name)
        target = root / relative
        require(target.is_file() and not target.is_symlink() and sha(target) == digest,
                f'raw manifest mismatch: {target}')
    actual = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file() and not p.is_symlink()
              and p != root / 'SHA256SUMS'}
    require({str(Path(n)) for n in names} == actual, 'raw manifest completeness differs')
    links = {str(p.relative_to(root)): os.readlink(p) for p in root.rglob('*') if p.is_symlink()}
    expected_links = ({f'cells/tika-cpu{cpu}-c{c}-r{r}-{arm}/run/bridge/tesseract': '/opt/indra/tesseract'
                       for cpu, c, r, arm in itertools.product([1, 2], [1, 2, 3], range(10),
                                                              ['ordinary', 'continuous'])}
                      if layer == 'tika' else {})
    require(links == expected_links, 'unexpected bridge alias type, path or target')
    return len(names)


def describe(values):
    values = sorted(values)
    require(len(values) == 10, 'exactly ten paired repetitions required')
    return {'count': len(values), 'min': float(values[0]),
            'median': float(statistics.median(values)), 'max': float(values[-1])}


def screen(ratios, concurrency):
    return {'throughput': concurrency == 1 or statistics.median(ratios['throughput']) >= Fraction(105, 100),
            'p95': statistics.median(ratios['p95']) <= Fraction(105, 100),
            'cpu_per_success': statistics.median(ratios['cpu_per_success']) <= Fraction(105, 100),
            'every_pair_memory_peak': max(ratios['memory_peak']) <= Fraction(125, 100)}


def timestamp(value):
    match = re.fullmatch(r'(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,9}))?Z', value)
    require(match is not None, 'invalid observed Docker timestamp')
    seconds = calendar.timegm(datetime.strptime(match[1], '%Y-%m-%dT%H:%M:%S').timetuple())
    return seconds * 10**9 + int((match[2] or '').ljust(9, '0'))


def frozen_schedule():
    result = []
    for repetition, cpus, concurrency, layer in itertools.product(range(10), [1, 2], [1, 2, 3],
                                                                  ['native', 'tika']):
        for arm in (['ordinary', 'continuous'] if repetition % 2 == 0 else ['continuous', 'ordinary']):
            result.append((layer, cpus, concurrency, repetition, arm))
    return result


def live_admission(directory, before, running, after, cid, cpus, layer, key):
    match = re.fullmatch(rf'/indra-continuous-{key}-([1-9][0-9]*)', before['Name'])
    require(match is not None, 'unexpected controller PID or assigned cell key in name')
    controller = match[1]
    evidence = [m for m in before['Mounts'] if m['Destination'] == '/evidence']
    require(len(evidence) == 1 and evidence[0]['Source'].endswith('/cells/' + directory.name),
            'original evidence mount identity differs')
    original = evidence[0]['Source'].rsplit('/cells/', 1)[0]
    owner = hashlib.sha256(f'{original}\0{SOURCE}\0{controller}'.encode()).hexdigest()
    for snapshot in [before, running, after]:
        require(snapshot['Id'] == cid and snapshot['Image'] == IMAGE
                and snapshot['Name'] == before['Name']
                and snapshot['Config']['Labels'].get('io.indra.continuous.owner') == owner,
                'owned before/running/final container identity differs')
    pid = running['State']['Pid']
    require(type(pid) is int and pid > 0 and running['State']['Running'] is True
            and before['State']['Running'] is False and before['State']['Pid'] == 0
            and after['State']['Pid'] == 0, 'invalid live host PID or process states')
    status = {}
    for line in (directory / 'host-process.status').read_text().splitlines():
        name, _, value = line.partition(':'); require(name not in status, 'duplicate host status field')
        status[name] = value.strip()
    require(status.get('Pid') == str(pid), 'live host PID differs from inspected process')
    require(status.get('Cpus_allowed_list') == ('0' if cpus == 1 else '0-1'),
            'actual process affinity differs')
    expected = ''.join(f'{cpu}\n' for cpu in range(cpus))
    for name in ['expected-cpus.txt', 'effective-cpus.txt', 'process-cpus.txt']:
        require((directory / name).read_text() == expected, 'actual effective/process CPU set differs')
    limits = re.findall(r'^Max stack size\s+(\S+)\s+(\S+)\s+bytes[ \t]*$',
                        (directory / 'host-process.limits').read_text(), re.M)
    require(limits == [('8388608', '8388608')], 'actual host process stack differs')
    group = (directory / 'host-process.cgroup').read_text().strip()
    require(group.startswith('0::/') and '\n' not in group, 'invalid live host cgroup record')
    relative = group[3:]
    require(all(part not in ['.', '..'] for part in Path(relative).parts), 'unsafe stored cgroup path')
    leaf = '/sys/fs/cgroup' + relative
    require(Path(relative).name == f'docker-{cid}.scope', 'live host cgroup does not bind owned CID')
    rows = (directory / 'host-ancestors.txt').read_text().splitlines(); observed = []; current = None
    fields = set()
    for row in rows:
        if row.startswith('/sys/fs/cgroup'):
            if current is not None:
                require(current == '/sys/fs/cgroup' or fields == {'cpu.max', 'memory.max'},
                        'incomplete non-root ancestor evidence')
            current = row; fields = set(); observed.append(current)
        else:
            parts = row.split(); require(current is not None and parts and parts[0] not in fields,
                                        'duplicate or unbound ancestor counter')
            fields.add(parts[0])
            if parts[0] == 'cpu.max':
                require(len(parts) == 3 and parts[2].isdigit() and int(parts[2]) > 0,
                        'invalid ancestor CPU counter')
                require(parts[1] == 'max' or (parts[1].isdigit()
                        and int(parts[1]) >= cpus * int(parts[2])), 'ancestor CPU quota too small')
            elif parts[0] == 'memory.max':
                require(len(parts) == 2 and (parts[1] == 'max' or
                        (parts[1].isdigit() and int(parts[1]) >= MEMORY)), 'ancestor memory too small')
            else:
                raise ValueError('unexpected ancestor field')
    paths = []; path = Path(leaf)
    for _ in range(64):
        paths.append(str(path))
        if path == Path('/sys/fs/cgroup'): break
        path = path.parent
    require(observed == paths and paths[-1] == '/sys/fs/cgroup',
            'live cgroup ancestor walk is missing, reordered or unowned')
    created, started, finished = (timestamp(before['Created']), timestamp(running['State']['StartedAt']),
                                  timestamp(after['State']['FinishedAt']))
    require(all(s['Created'] == before['Created'] for s in [running, after])
            and after['State']['StartedAt'] == running['State']['StartedAt']
            and created <= started < finished, 'inconsistent observed execution timestamps')
    return {'container_id': cid, 'owner': owner, 'controller_pid': int(controller),
            'host_pid': pid, 'host_cgroup': relative, 'created_ns': created,
            'started_ns': started, 'finished_ns': finished}


def cell(root, record, layer, identity, key):
    cpus, concurrency, repetition, arm = (record[k] for k in
                                         ['cpus', 'concurrency', 'repetition', 'arm'])
    require(type(cpus) is int and cpus in [1, 2] and type(concurrency) is int
            and concurrency in [1, 2, 3] and type(repetition) is int
            and 0 <= repetition < 10 and arm in ['ordinary', 'continuous'], 'invalid cell key')
    relative = Path(f'cells/{layer}-cpu{cpus}-c{concurrency}-r{repetition}-{arm}')
    require(record['summary'] == str(relative / 'run/summary.json'), 'unexpected cell path')
    directory = root / relative
    run = directory / 'run'
    summary = load(run / 'summary.json')
    require(summary['complete'] is True and summary['planned_requests'] == 12
            and summary['successful_requests'] == 12 and summary['cancellation_probe'] is False
            and all(summary[k] == record[k] for k in ['cpus', 'concurrency', 'arm'])
            and number(summary['wall_us'], True), 'incomplete or wrong cell summary')
    if layer == 'tika':
        require(summary['child_cleanup_complete'] is True and summary['terminal_error'] is None,
                'Tika children or terminal failure incomplete')
    requests = [json.loads(row) for row in (run / 'requests.jsonl').read_text().splitlines()]
    require(len(requests) == 12, 'exactly twelve request outcomes required')
    latencies = []
    for i, request in enumerate(requests):
        require(request['sequence'] == i and request['fixture'] == SCHEDULE[i]
                and request['status'] == 'success' and number(request['elapsed_us'], True),
                'missing, wrong or unsuccessful request outcome')
        digest = request['output_sha256']
        if isinstance(digest, list):
            require(len(digest) == 32 and all(type(x) is int and 0 <= x <= 255 for x in digest),
                    'invalid body digest bytes')
            digest = bytes(digest).hex()
        reference = run / f'reference-{SCHEDULE[i]}.txt'
        require(digest == sha(reference) and request['output_bytes'] == reference.stat().st_size,
                'reference body hash or length differs')
        require(layer != 'tika' or request['http_status'] == 200, 'non-success HTTP response')
        latencies.append(request['elapsed_us'])
    latencies.sort()
    p95 = latencies[math.ceil(0.95 * len(latencies)) - 1]
    reported = load(run / 'latency.json') if layer == 'native' else summary
    require(reported['p95_us'] == p95 and reported['p50_us'] == latencies[5],
            'reported nearest-rank quantile differs')
    before, running, after = (load(directory / f) for f in
                               ['container-before.json', 'container-running.json', 'container.json'])
    require(len(before) == len(running) == len(after) == 1, 'ambiguous container identity')
    before, running, after = before[0], running[0], after[0]
    cid = (directory / 'container-id.txt').read_text().strip()
    require(re.fullmatch('[0-9a-f]{64}', cid) is not None
            and before['Id'] == after['Id'] == cid and before['Image'] == after['Image'] == IMAGE,
            'container image or full owned CID differs')
    live = live_admission(directory, before, running, after, cid, cpus, layer, key)
    config = before['HostConfig']
    require(config['NanoCpus'] == cpus * 1000000000
            and config['CpusetCpus'] == ('0' if cpus == 1 else '0,1')
            and config['Memory'] == config['MemorySwap'] == MEMORY
            and config['PidsLimit'] == 128 and config['CgroupnsMode'] == 'private'
            and config['CapDrop'] == ['ALL'] and not config.get('CapAdd')
            and not config['Privileged'] and not config['PidMode']
            and config['NetworkMode'] == 'none' and config['ReadonlyRootfs']
            and config['SecurityOpt'] == ['no-new-privileges'], 'container resources differ')
    require(any(v == {'Name': 'stack', 'Soft': 8388608, 'Hard': 8388608}
                for v in config['Ulimits']), 'native stack limit differs')
    state = after['State']
    require(state['ExitCode'] == 0 and state['OOMKilled'] is False and state['Running'] is False,
            'container failed or OOM killed')
    require((directory / 'removed.txt').read_text().strip() == cid
            and (directory / 'remaining.txt').read_bytes() == b'', 'owned cleanup incomplete')
    for name, expected in [('cpu.max', f'{cpus * 100000} 100000'), ('memory.max', str(MEMORY)),
                           ('memory.swap.max', '0'), ('pids.max', '128')]:
        require((directory / f'host-{name}').read_text().strip() == expected,
                f'actual host leaf budget differs: {name}')
    phases = [run / 'measurement-before', run / 'measurement-after']
    usage, throttle = [], []
    for phase in phases:
        stats = counters(phase / 'cpu.stat'); events = counters(phase / 'memory.events')
        usage.append(stats['usage_usec']); throttle.append(stats['throttled_usec'])
        require(events['oom'] == events['oom_kill'] == 0, 'cgroup OOM event')
        require((phase / 'memory.max').read_text().strip() == str(MEMORY)
                and (phase / 'memory.swap.max').read_text().strip() == '0', 'phase budget differs')
    require(usage[1] > usage[0] and throttle[1] >= throttle[0], 'CPU counter decreased or empty')
    peak = int((phases[1] / 'memory.peak').read_text())
    require(0 < peak <= MEMORY, 'invalid or excessive memory peak')
    if layer == 'tika':
        assets = [json.loads(line) for line in (run / 'assets.jsonl').read_text().splitlines()]
        require(len(assets) == 5 and {a['role'] for a in assets} == set(identity['role_sha256'])
                and all(a['sha256'] == identity['role_sha256'][a['role']] for a in assets),
                'actual Tika roles differ')
    return {'layer': layer, 'cpus': cpus, 'concurrency': concurrency, 'repetition': repetition,
            'arm': arm, 'summary_sha256': sha(run / 'summary.json'),
            'requests_sha256': sha(run / 'requests.jsonl'), 'successful_requests': 12,
            'wall_us': summary['wall_us'], 'throughput_per_second': 12000000 / summary['wall_us'],
            'p95_us': p95, 'measurement_cpu_us': usage[1] - usage[0],
            'measurement_throttled_us': throttle[1] - throttle[0],
            'process_memory_peak_bytes': peak, 'observed_execution': live}


def analyze(native, tika):
    cells = {}; inputs = []; layer_executions = {}
    for layer, root in [('native', native), ('tika', tika)]:
        entries = manifest(root, layer); index = load(root / 'study-index.json')
        require(index['assigned_layer'] == layer and index['assigned_layer_complete'] is True
                and index['controller_exit'] == 1 and index['cleanup_exit'] == 0,
                'intentional per-layer exit1 or assigned-layer completion differs')
        observed_keys = [tuple(r[k] for k in ['layer', 'cpus', 'concurrency', 'repetition', 'arm'])
                         for r in index['cells']]
        require(observed_keys == frozen_schedule(), 'frozen complete schedule differs')
        assigned = [(key, r) for key, r in enumerate(index['cells']) if r['layer'] == layer]
        recorded_schedule = [line.split('\t') for line in
                             (root / 'control/assigned-schedule.tsv').read_text().splitlines()]
        expected_schedule = [[str(key), str(r['cpus']), str(r['concurrency']), str(r['repetition']), r['arm']]
                             for key, r in assigned]
        require(recorded_schedule == expected_schedule, 'assigned schedule differs')
        records = [r for _, r in assigned]
        require(len(records) == 120 and all(r['status'] == 'completed' and r['exit_code'] == 0
                                          for r in records), 'failed or missing assigned cells')
        require(all(r['status'] == 'not_executed' for r in index['cells'] if r['layer'] != layer),
                'another layer was relabeled')
        control = root / 'control'; identity = load(control / 'build-identity.json')
        require(sha(control / 'build-identity.json') == IDENTITIES[layer]
                and sha(control / 'native-equivalence-receipt') == NATIVE_RECEIPT
                and identity['source_sha'] == SOURCE and identity['binary_sha256'] == DRIVERS[layer]
                and identity['protocol_sha256'] == PROTOCOL
                and sha(control / 'continuous-protocol.json') == PROTOCOL
                and sha(control / 'models.sha256') == MODELS
                and identity['native_equivalence_passed'] is True
                and sha(control / 'native-equivalence-receipt') == identity['native_equivalence_receipt_sha256'],
                'frozen source, protocol, driver, model or native admission differs')
        previous = None; owner = None; execution = []
        for assigned_key, record in assigned:
            value = cell(root, record, layer, identity, assigned_key)
            live = value['observed_execution']
            require(previous is None or previous <= live['created_ns'],
                    'assigned observed execution order overlaps or is reversed')
            require(owner is None or owner == live['owner'], 'layer controller ownership changed')
            previous = live['finished_ns']; owner = live['owner']; execution.append(live)
            key = tuple(value[k] for k in ['layer', 'cpus', 'concurrency', 'repetition', 'arm'])
            require(key not in cells, 'duplicate paired cell'); cells[key] = value
        layer_executions[layer] = execution
        inputs.append({'layer': layer, 'manifest_entries': entries,
                       'raw_inventory_sha256': sha(root / 'SHA256SUMS'),
                       'study_index_sha256': sha(root / 'study-index.json'),
                       'controller_exit': 1, 'cleanup_exit': 0})
    require(layer_executions['native'][-1]['finished_ns'] <= layer_executions['tika'][0]['created_ns'],
            'native and Tika observed execution windows overlap')
    expected = set(itertools.product(['native', 'tika'], [1, 2], [1, 2, 3], range(10),
                                    ['ordinary', 'continuous']))
    require(set(cells) == expected, 'paired schedule coverage differs')
    pairs = []; strata = []
    for layer, cpus, concurrency in itertools.product(['native', 'tika'], [1, 2], [1, 2, 3]):
        ratios = {key: [] for key in ['throughput', 'p95', 'cpu_per_success', 'memory_peak']}
        differences = {key: [] for key in ratios}
        for repetition in range(10):
            a = cells[layer, cpus, concurrency, repetition, 'ordinary']
            b = cells[layer, cpus, concurrency, repetition, 'continuous']
            raw = {'throughput': Fraction(a['wall_us'], b['wall_us']),
                   'p95': Fraction(b['p95_us'], a['p95_us']),
                   'cpu_per_success': Fraction(b['measurement_cpu_us'], a['measurement_cpu_us']),
                   'memory_peak': Fraction(b['process_memory_peak_bytes'], a['process_memory_peak_bytes'])}
            delta = {'throughput': Fraction(12000000, b['wall_us']) - Fraction(12000000, a['wall_us']),
                     'p95': b['p95_us'] - a['p95_us'],
                     'cpu_per_success': Fraction(b['measurement_cpu_us'] - a['measurement_cpu_us'], 12),
                     'memory_peak': b['process_memory_peak_bytes'] - a['process_memory_peak_bytes']}
            for key in ratios: ratios[key].append(raw[key]); differences[key].append(delta[key])
            pairs.append({'layer': layer, 'cpus': cpus, 'concurrency': concurrency,
                          'repetition': repetition,
                          'order': ['ordinary', 'continuous'] if repetition % 2 == 0 else ['continuous', 'ordinary'],
                          'ordinary': a, 'continuous': b,
                          'ratios': {key: float(value) for key, value in raw.items()},
                          'differences': {key: float(value) for key, value in delta.items()}})
        gates = screen(ratios, concurrency)
        strata.append({'layer': layer, 'cpus': cpus, 'concurrency': concurrency,
                       'ratios': {key: describe(values) for key, values in ratios.items()},
                       'differences': {key: describe(values) for key, values in differences.items()},
                       'throughput_gate_applies': concurrency in [2, 3], 'gates': gates,
                       'screen_passed': all(gates.values())})
    return {'schema_version': 1, 'source_sha': SOURCE, 'protocol_sha256': PROTOCOL,
            'inputs': inputs, 'admitted_cells': len(cells), 'successful_requests': len(cells) * 12,
            'paired_repetitions': len(pairs),
            'scoped_prior_native_gate_receipt_sha256': NATIVE_RECEIPT,
            'stored_live_resource_ownership_and_assigned_execution_order_verified': True,
            'screen_rule': 'Exact rational comparisons: median paired throughput>=1.05 at concurrency2/3; median paired p95 and CPU-use-per-success<=1.05 at every concurrency; every paired memory peak<=1.25 and absolute3GiB. All added-error/ownership/native/lifecycle gates remain required.',
            'engineering_screen_passed': all(s['screen_passed'] for s in strata),
            'default_promotion': False,
            'decision_scope': 'Synthetic mixed-build implementation pilot remains opt-in even on screen pass. No population confidence interval, stable tail estimate or universal optimum.',
            'resource_scope': 'CPU and throttling are measurement-after minus measurement-before in each cell, after load and warmup. memory.peak is the fresh cgroup high-water mark at the final recorded measurement read and includes startup/warmup; cached file-page ownership can predate the cell. Native copy bytes and child/native allocations are not inferred from Rust driver counters.',
            'strata': strata, 'pairs': pairs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('native', type=Path); parser.add_argument('tika', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    require(not args.output.exists() and not args.output.is_symlink(), 'output must be new')
    result = analyze(args.native.resolve(strict=True), args.tika.resolve(strict=True))
    args.output.mkdir(mode=0o700, parents=True)
    (args.output / 'paired.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'admitted_cells': result['admitted_cells'], 'pairs': result['paired_repetitions'],
                      'engineering_screen_passed': result['engineering_screen_passed'],
                      'default_promotion': False}))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f'continuous admission failed: {error}', file=sys.stderr)
        sys.exit(1)
