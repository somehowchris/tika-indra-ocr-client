#!/usr/bin/env python3
"""Exercise portable evidence admission and rejection of modified raw records."""
import importlib.util
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

MODULE_PATH = Path(__file__).with_name('admit-native-ablations.py')
spec = importlib.util.spec_from_file_location('admission', MODULE_PATH)
admission = importlib.util.module_from_spec(spec)
spec.loader.exec_module(admission)
PACKET = Path(sys.argv.pop(1))


class EvidenceAdmission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix='indra-ablation-evidence-')
        cls.root = Path(cls.directory.name)
        with tarfile.open(PACKET / 'raw-records.tar.gz') as archive:
            archive.extractall(cls.root, filter='data')
        for row in (PACKET / 'portable-records.sha256').read_text().splitlines():
            digest, name = row.split('  ', 1)
            if admission.sha(cls.root / name) != digest:
                raise ValueError(f'portable byte integrity failure: {name}')
        cls.records = cls.root / 'run'
        cls.phase = cls.records / 'cell-1/cpu-1/ablation'

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def modify_and_reject(self, path, change):
        original = path.read_bytes()
        try:
            path.write_bytes(change(original))
            with self.assertRaises(ValueError):
                admission.admit(self.records)
        finally:
            path.write_bytes(original)

    def test_complete_portable_records(self):
        result = admission.admit(self.records)
        self.assertTrue(result['admitted'])
        expected = json.loads((PACKET / 'admission.json').read_text())
        expected.pop('original_materials')
        self.assertEqual(result, expected)

    def test_quota_change_is_rejected(self):
        self.modify_and_reject(self.phase / 'before/cpu.max', lambda _: b'200000 100000\n')

    def test_oom_is_rejected(self):
        self.modify_and_reject(self.phase / 'after/memory.events', lambda b: b.replace(b'oom_kill 0', b'oom_kill 1'))

    def test_output_change_is_rejected(self):
        self.modify_and_reject(self.phase / 'output.txt', lambda b: b + b'!')

    def test_request_loss_is_rejected(self):
        self.modify_and_reject(self.phase / 'requests.jsonl', lambda b: b'\n'.join(b.splitlines()[:-1])+b'\n')

    def test_wrong_settings_are_rejected(self):
        self.modify_and_reject(self.phase / 'requests.jsonl', lambda b: b.replace(b'"batch_size":64', b'"batch_size":8', 1))

    def test_host_pid_change_is_rejected(self):
        def changed(raw):
            data = json.loads(raw)
            data['host_pid'] += 1
            return json.dumps(data).encode()
        self.modify_and_reject(self.phase / 'host-admission.json', changed)

    def test_missing_removal_is_rejected(self):
        self.modify_and_reject(self.phase / 'removed.txt', lambda _: b'')

    def test_pair_arithmetic_change_is_rejected(self):
        def changed(raw):
            data = json.loads(raw)
            data['pairs'][0]['median_ratio'] += 0.1
            return json.dumps(data).encode()
        self.modify_and_reject(self.records / 'analysis.json', changed)


if __name__ == '__main__':
    unittest.main()
