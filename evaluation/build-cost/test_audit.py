import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('audit', ROOT/'audit.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditTests(unittest.TestCase):
    def parse(self, text):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'log'
            path.write_text(text)
            return audit.parse_log(path)

    def test_invocations_cannot_reuse_old_vertex_identity(self):
        data = self.parse('#0 building with builder\n#1 [native-build 6/6] RUN build-native-runtime.sh /native\n#1 DONE 2.5s\n#0 building with builder\n#1 [native-build 6/6] RUN build-native-runtime.sh /native\n#1 CACHED\n')
        self.assertEqual(len(data['phases']), 2)
        self.assertEqual([p['invocation'] for p in data['phases']], [0, 1])
        self.assertEqual([p['elapsed_seconds'] for p in data['phases']], [2.5, None])

    def test_incomplete_failed_ansi_and_timestamp(self):
        data = self.parse('2026-10-18T00:00:00.000Z \x1b[32m#1 [native-build 6/6] RUN build-native-runtime.sh /native\x1b[0m\n#2 importing cache manifest from gha:x\n#2 ERROR: missing blob not found\n')
        self.assertEqual([p['state'] for p in data['phases']], ['incomplete', 'failed'])
        self.assertEqual(data['phases'][0]['start_log_time'], '2026-10-18T00:00:00.000Z')
        self.assertEqual(len(data['identity_and_failure_lines']), 1)

    def test_repeated_elapsed_updates_are_not_summed(self):
        data = self.parse('#3 exporting to docker image format\n#3 DONE 2.0s\n#3 DONE 3.0s\n')
        self.assertEqual(data['phases'][0]['elapsed_seconds'], 3.0)
        self.assertEqual(len(data['phases'][0]['terminal_updates']), 2)

    def test_actual_nonzero_command_is_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)/'phase'
            done = subprocess.run([sys.executable, '-B', str(ROOT/'audit.py'), 'record-phase', '--phase', 'test-failure', '--output', str(output), '--', sys.executable, '-c', 'print("retained"); raise SystemExit(7)'], capture_output=True)
            self.assertEqual(done.returncode, 7, done.stderr)
            receipt = json.loads((output/'phase.json').read_text())
            self.assertEqual(receipt['exit_code'], 7)
            self.assertGreater(receipt['elapsed_ns'], 0)
            self.assertEqual((output/'stdout').read_text(), 'retained\n')
            self.assertEqual(receipt['stdout_sha256'], audit.sha(output/'stdout'))

    def test_inventory_rejects_links_and_binds_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root/'source'
            path.write_text('exact bytes')
            first = audit.inventory(root)
            path.chmod(0o700)
            self.assertNotEqual(first, audit.inventory(root))
            (root/'alias').symlink_to(path)
            with self.assertRaises(ValueError):
                audit.inventory(root)

    def test_missing_binary_failure_is_retained(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            command = [sys.executable, '-B', str(ROOT/'audit.py'), 'compare-pair']
            for role in ('baseline', 'candidate', 'baseline-sdk', 'candidate-sdk', 'baseline-materials', 'candidate-materials'):
                directory = root/role
                directory.mkdir()
                command += ['--'+role, str(directory)]
            command += ['--output', str(root/'result')]
            done = subprocess.run(command, capture_output=True)
            self.assertEqual(done.returncode, 1)
            receipt = json.loads((root/'result/comparison.json').read_text())
            self.assertIs(receipt['pass'], False)
            self.assertIn('missing regular executable', receipt['failure'])

    def test_real_retained_logs(self):
        expected = {'39114744717': ('done', 4416.3), '39115648687': ('cached', None), '39143910423': ('cached', None)}
        for run, (state, elapsed) in expected.items():
            report = audit.parse_log(ROOT/'evidence'/f'{run}-service.log.gz')
            native = [p for p in report['phases'] if p['phase'] == 'native-build-combined']
            self.assertEqual(len(native), 1)
            self.assertEqual((native[0]['state'], native[0]['elapsed_seconds']), (state, elapsed))


if __name__ == '__main__':
    unittest.main()
