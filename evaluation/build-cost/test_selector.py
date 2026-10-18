"""Adversarial selector fixtures only; not compilation/equivalence evidence."""
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT/'scripts/select-cargo-artifact.py'
spec = importlib.util.spec_from_file_location('selector', SCRIPT)
selector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selector)


class SelectorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        self.binary = self.root/'exact-artifact'
        self.binary.write_bytes(b'\x7fELFsynthetic-selector-fixture-not-valid-native-evidence')
        self.binary.chmod(0o755)
        self.package = 'path+file:///app/runtime/crates/indra-engine#0.2.0'
        self.event = {'reason': 'compiler-artifact', 'package_id': self.package,
                      'target': {'name': 'native', 'kind': ['test']}, 'profile': {'test': True},
                      'executable': str(self.binary), 'filenames': [str(self.binary)]}

    def select(self, events=None, package=None, name='native', kind='test', test=True):
        if events is None:
            events = [self.event, {'reason': 'build-finished', 'success': True}]
        return selector.select(io.StringIO(''.join(json.dumps(e)+'\n' for e in events)), package or self.package, name, kind, test, self.root)

    def test_exact_and_fresh_artifact(self):
        for fresh in (True, False):
            self.event['fresh'] = fresh
            self.assertEqual(self.select()[0], self.binary)

    def test_all_seven_role_identity_shapes(self):
        for name, kind, test in [('indra-server', 'bin', False), ('indra_server', 'lib', True), ('lifecycle_native', 'test', True), ('ocr', 'example', False), ('acceptance', 'example', False), ('indra_engine', 'lib', True), ('native', 'test', True)]:
            with self.subTest(name=name):
                self.event['target'] = {'name': name, 'kind': [kind]}
                self.event['profile']['test'] = test
                self.assertEqual(self.select(name=name, kind=kind, test=test)[0], self.binary)

    def test_wrong_package_name_kind_or_test_never_selects(self):
        for kwargs in [{'package': self.package+'wrong'}, {'name': 'wrong'}, {'kind': 'bin'}, {'test': False}]:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                self.select(**kwargs)

    def test_ambiguity_and_failed_or_incomplete_builds_rejected(self):
        end = {'reason': 'build-finished', 'success': True}
        for events in [[], [self.event], [self.event, self.event, end], [self.event, {**end, 'success': False}], [self.event, {**end, 'success': 1}], [self.event, {**end, 'success': 'true'}], [self.event, end, end], [self.event, end, self.event]]:
            with self.subTest(events=len(events)), self.assertRaises(ValueError):
                self.select(events)

    def test_invalid_json_record_types_and_nonfinite_rejected(self):
        for raw in ['garbage\n', '\n', 'null\n', '[]\n', '1\n', '{}\n', '{"reason":2}\n', '{"reason":"compiler-message","value":NaN}\n']:
            with self.subTest(raw=raw), self.assertRaises((ValueError, TypeError)):
                selector.select(io.StringIO(raw), self.package, 'native', 'test', True, self.root)

    def test_wrong_file_and_path_kinds_rejected(self):
        for value in [None, 42, 'relative-path', str(self.root/'missing')]:
            self.event['executable'] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.select()
        self.event['executable'] = str(self.binary)
        self.binary.chmod(0o644)
        with self.assertRaises(ValueError):
            self.select()
        self.binary.chmod(0o755)
        self.binary.write_bytes(b'not ELF')
        with self.assertRaises(ValueError):
            self.select()

    def test_symlink_and_parent_escape_rejected(self):
        link = self.root/'link'
        link.symlink_to(self.binary)
        self.event.update(executable=str(link), filenames=[str(link)])
        with self.assertRaises(ValueError):
            self.select()
        self.event.update(executable=str(self.root/'..'/self.root.name/'exact-artifact'), filenames=[str(self.binary)])
        with self.assertRaises(ValueError):
            self.select()

    def test_filename_inventory_and_profile_types_rejected(self):
        for filenames in [[], str(self.binary), None, [42]]:
            self.event['filenames'] = filenames
            with self.subTest(filenames=filenames), self.assertRaises(ValueError):
                self.select()
        self.event['filenames'] = [str(self.binary)]
        self.event['profile']['test'] = 1
        with self.assertRaises(ValueError):
            self.select()

    def test_cli_copies_exact_bytes_exclusively_and_binds_input(self):
        stream = self.root/'cargo.jsonl'
        stream.write_text(json.dumps(self.event)+'\n'+json.dumps({'reason':'build-finished','success':True})+'\n')
        dest = self.root/'selected'
        receipt = self.root/'receipt.json'
        cmd = [sys.executable, '-B', str(SCRIPT), '--json', str(stream), '--package-id', self.package,
               '--name', 'native', '--kind', 'test', '--test', 'true', '--artifact-root', str(self.root),
               '--destination', str(dest), '--receipt', str(receipt)]
        first = subprocess.run(cmd, capture_output=True)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(dest.read_bytes(), self.binary.read_bytes())
        self.assertEqual(json.loads(receipt.read_text())['target'], self.event['target'])
        second = subprocess.run(cmd, capture_output=True)
        self.assertNotEqual(second.returncode, 0)
        self.assertEqual(dest.read_bytes(), self.binary.read_bytes())


if __name__ == '__main__':
    unittest.main()
