import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
source = json.loads((root / 'source-receipts.json').read_text())
fixed = source['fixed']['commit']
assert fixed == 'c405064013b4fcd146f8992e90b49d3cef873a8d'
assert subprocess.check_output(['git', '-C', source['fixed']['worktree'], 'rev-parse', 'HEAD'], text=True).strip() == fixed
assert not subprocess.check_output(['git', '-C', source['fixed']['worktree'], 'status', '--porcelain'], text=True).strip()
checks = root / 'fixed-checks-attempt3'
commands = [json.loads(line) for line in (checks / 'commands.jsonl').read_text().splitlines()]
assert all(record['status'] == 0 for record in commands)
baseline = [json.loads(line) for line in (root / 'before-fix/commands.jsonl').read_text().splitlines()]
counterexample = next(record for record in baseline if record['name'] == 'process')
assert counterexample['status'] == 101 and counterexample['expected_failure']
assert 'left: Null' in (root / 'before-fix/process.stderr').read_text()
assert '0000000000000001' in (root / 'before-fix/process.stderr').read_text()
elf = json.loads((checks / 'elf-receipts.json').read_text())
variants = {}
for variant in ['default-c405064-a3', 'sentry-only-c405064-a3']:
    selected = json.loads((checks / (variant + '-selection.json')).read_text())
    fresh = json.loads((checks / (variant + '-fresh-local-artifacts.json')).read_text())
    assert fresh and all(record['fresh'] is False for record in fresh)
    for item in selected.values():
        assert elf[Path(item['artifact']).name]['sha256'] == item['sha256']
        assert not elf[Path(item['artifact']).name]['interpreter']
        assert not elf[Path(item['artifact']).name]['dynamic_dependencies']
    features = selected['gateway']['cargo_artifact']['features']
    assert ('otlp' in features) == variant.startswith('default')
    variants[variant] = {
        'features': features,
        'target_dir': next(record['target_dir'] for record in commands
                           if record['name'] == ('build-default' if variant.startswith('default') else 'build-sentry-only')),
        'all_changed_local_artifacts_fresh_false': True,
        'artifacts': {role: {'path': item['artifact'], 'sha256': item['sha256']}
                      for role, item in selected.items()},
    }
processes = []
for mode, expected_cpu, expected_affinity in [
    ('process-default-1cpu', '100000 100000', '12'),
    ('process-default-2cpu', '200000 100000', '12-13'),
    ('process-sentry-only', '200000 100000', '12-13'),
]:
    directory = root / (mode + '-attempt3')
    records = [json.loads(line) for line in (directory / 'commands.jsonl').read_text().splitlines()]
    process = next(record for record in records if record['name'] == 'process')
    assert process['status'] == 0
    limits = json.loads((directory / 'cgroup-before.json').read_text())
    assert limits['cpu.max'].strip() == expected_cpu
    assert limits['cpuset.cpus.effective'].strip() == expected_affinity
    assert limits['memory.max'].strip() == '3221225472'
    assert limits['memory.swap.max'].strip() == '0'
    after = json.loads((directory / 'cgroup-after.json').read_text())
    assert 'oom_kill 0' in after['memory.events']
    inspection = json.loads((root / (mode + '-attempt3-inspect-after.json')).read_text())[0]
    host = inspection['HostConfig']
    assert not host['Privileged'] and host['NetworkMode'] == 'none'
    assert inspection['State']['ExitCode'] == 0 and not inspection['State']['OOMKilled']
    assert all('docker.sock' not in mount.get('Source', '') for mount in inspection['Mounts'])
    processes.append({'mode': mode, 'status': 0, 'command': process['command'],
                      'cpu_max': expected_cpu, 'affinity': expected_affinity,
                      'memory_bytes': 3221225472, 'swap_bytes': 0,
                      'memory_peak_bytes': int(after['memory.peak'].strip()),
                      'pids_peak': int(after['pids.peak'].strip()),
                      'gateway_binary_binding': 'process() explicitly sets INDRA_TEST_GATEWAY_BIN to the matching selected /evidence/bin artifact'})
summary = {
    'passed': True,
    'scope': 'Unmeasured gateway/telemetry correctness; no OCR inference, DOCS_RS, native C++ rebuild, external network, privileged container, host networking or socket mount',
    'sources': source,
    'counterexample': {'producer': source['before']['commit'], 'status': 101,
                       'observed': 'Sentry tags.request_id Null versus returned X-Request-ID 0000000000000001',
                       'stderr': 'before-fix/process.stderr'},
    'unit_tests': {'reporter_telemetry': 4, 'gateway': 11},
    'strict_clippy': 'all features/all targets; -D warnings; passed',
    'workspace_fmt_check': 'passed',
    'variants': variants,
    'process_tests': processes,
    'elf': {'target': 'x86_64-unknown-linux-gnu', 'rustflags': '-Ctarget-feature=+crt-static',
            'final_variant_interpreters': False, 'final_variant_needed_entries': False,
            'receipts': 'fixed-checks-attempt3/elf-receipts.json'},
    'retained_failed_attempts': [
        'preflight-failures.json',
        'before-fix-container.stderr: evidence directory denied before compilation',
        'fixed-checks/clippy.stderr: initial rustup launcher attempted an unavailable network sync',
        'fixed-checks/clean-owned-crates.stderr: initial non-release clean removed0 files; those artifact selections are invalidated',
    ],
    'accepted_artifact_rule': 'Only the two c405064-a3 variants above are accepted fixed artifacts; older bin/default-* and bin/sentry-only-* copies remain solely as invalidated failed-attempt evidence',
    'commands_and_status': 'Per-stage commands.jsonl and distinct container-command/inspect-before/inspect-after JSON files',
}
(root / 'validation-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
