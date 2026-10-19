"""Arithmetic boundaries and malformed evidence rejection; no OCR execution."""
from fractions import Fraction
import importlib.util
import hashlib
import json
import shutil
import subprocess
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    'continuous_admission', Path(__file__).resolve().parents[1] / 'scripts/admit-continuous-pilot.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class Admission(unittest.TestCase):
    def test_exact_thresholds_and_single_memory_regression(self):
        ratios = {'throughput': [Fraction(105, 100)] * 10,
                  'p95': [Fraction(105, 100)] * 10,
                  'cpu_per_success': [Fraction(105, 100)] * 10,
                  'memory_peak': [Fraction(125, 100)] * 10}
        self.assertTrue(all(MODULE.screen(ratios, 2).values()))
        ratios['memory_peak'][9] += Fraction(1, 10**9)
        self.assertFalse(MODULE.screen(ratios, 2)['every_pair_memory_peak'])
        ratios['memory_peak'][9] -= Fraction(1, 10**9)
        ratios['throughput'][0] -= Fraction(1, 100)
        ratios['throughput'][1] -= Fraction(1, 100)
        ratios['throughput'][2] -= Fraction(1, 100)
        ratios['throughput'][3] -= Fraction(1, 100)
        ratios['throughput'][4] -= Fraction(1, 100)
        self.assertFalse(MODULE.screen(ratios, 2)['throughput'])
        self.assertTrue(MODULE.screen(ratios, 1)['throughput'])

    def test_median_uses_paired_ratios_not_ratio_of_unpaired_medians(self):
        ratios = [Fraction(a, b) for a, b in [(1, 10), (10, 1)] * 5]
        self.assertEqual(MODULE.describe(ratios)['median'], 5.05)
        with self.assertRaisesRegex(ValueError, 'ten paired'):
            MODULE.describe(ratios[:-1])

    def test_duplicate_and_malformed_counters_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'cpu.stat'
            for bad in ['usage_usec 2\nusage_usec 3\n', 'usage_usec NaN\n', 'usage_usec -1\n']:
                path.write_text(bad)
                with self.assertRaisesRegex(ValueError, 'counter'):
                    MODULE.counters(path)

    def test_manifest_rejects_unlisted_file_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            file = root / 'input'; file.write_bytes(b'actual evidence')
            manifest = root / 'SHA256SUMS'
            manifest.write_text(f'{MODULE.sha(file)}  ./input\n')
            self.assertEqual(MODULE.manifest(root, 'native'), 1)
            extra = root / 'unlisted'; extra.write_bytes(b'not inventoried')
            with self.assertRaisesRegex(ValueError, 'completeness'):
                MODULE.manifest(root, 'native')
            extra.unlink(); extra.symlink_to('/missing/private/path')
            with self.assertRaisesRegex(ValueError, 'alias'):
                MODULE.manifest(root, 'native')


    def test_packet_manifest_files_are_git_tracked(self):
        root = Path(__file__).resolve().parents[1]
        if not (root / '.git').exists():
            self.skipTest('Git archive has no index; tracking is checked before archive production')
        packet = root / 'evaluation/continuous/evidence/2026-10-19-pilot'
        for row in (packet / 'SHA256SUMS').read_text().splitlines():
            _, name = row.split('  ', 1)
            subprocess.run(['git', '-C', str(root), 'ls-files', '--error-unmatch', '--',
                            str((packet / name).relative_to(root))], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

    def test_rehashed_live_record_mutations_fail_the_specific_admission(self):
        root = Path(__file__).resolve().parents[1]
        packet = root / 'evaluation/continuous/evidence/2026-10-19-pilot'
        with tempfile.TemporaryDirectory() as temporary:
            unpacked = Path(temporary) / 'original'; unpacked.mkdir()
            subprocess.run(['tar', '-xzf', str(packet / 'raw.tar.gz'), '-C', str(unpacked)], check=True)
            native_name = 'continuous-native-85fef71-20261019'
            original = unpacked / native_name
            first = 'cells/native-cpu1-c1-r0-ordinary/'
            second = 'cells/native-cpu1-c1-r0-continuous/'
            def mutate_json(path, transform):
                value = json.loads(path.read_text()); transform(value)
                path.write_text(json.dumps(value))
            def identity(path):
                mutate_json(path, lambda rows: rows[0]['Config']['Labels'].__setitem__(
                    'io.indra.continuous.owner', '0' * 64))
            def pid(path):
                mutate_json(path, lambda rows: rows[0]['State'].__setitem__(
                    'Pid', rows[0]['State']['Pid'] + 1))
            def replace(path, old, new):
                text = path.read_text(); self.assertIn(old, text)
                replacement = path.with_name(path.name + '.changed')
                replacement.write_text(text.replace(old, new, 1))
                replacement.replace(path)
            def order(path):
                old_created = json.loads((original / first / 'container-before.json').read_text())[0]['Created']
                for name in ['container-before.json', 'container-running.json', 'container.json']:
                    mutate_json(path.parent / name, lambda rows: rows[0].__setitem__('Created', old_created))
            cases = [
                (first + 'container-before.json', identity, 'owned before/running/final'),
                (first + 'container-running.json', pid, 'live host PID differs'),
                (first + 'host-process.cgroup', lambda p: replace(p, 'docker-', 'other-'), 'live host cgroup'),
                (first + 'host-ancestors.txt', lambda p: replace(p, 'cpu.max 100000 100000', 'cpu.max 50000 100000'), 'ancestor CPU'),
                (first + 'host-process.status', lambda p: replace(p, 'Cpus_allowed_list:\t0', 'Cpus_allowed_list:\t1'), 'actual process affinity'),
                (first + 'host-process.limits', lambda p: replace(p, '8388608', '4194304'), 'actual host process stack'),
                (second + 'container-before.json', order, 'assigned observed execution order'),
                ('control/assigned-schedule.tsv', lambda p: p.write_text('\n'.join(reversed(p.read_text().splitlines()))+'\n'), 'assigned schedule'),
            ]
            for index, (name, transform, message) in enumerate(cases):
                with self.subTest(record=name):
                    changed = Path(temporary) / f'changed{index}'
                    shutil.copytree(original, changed, symlinks=True)
                    transform(changed / name)
                    # Rebuild the original regular-file manifest so the failure
                    # must come from the concrete live/order gate, not integrity.
                    lines = []
                    for row in (changed / 'SHA256SUMS').read_text().splitlines():
                        _, relative = row.split('  ', 1)
                        with (changed / relative).open('rb') as stream:
                            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
                        lines.append(f'{digest}  {relative}\n')
                    (changed / 'SHA256SUMS').write_text(''.join(lines))
                    with self.assertRaisesRegex(ValueError, message):
                        MODULE.analyze(changed, unpacked / 'continuous-tika-85fef71-20261019')
                    shutil.rmtree(changed)


if __name__ == '__main__':
    unittest.main()
