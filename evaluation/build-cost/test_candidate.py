"""Freeze original compile contracts and native-stage inputs for the candidate."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]
RECEIPT = ROOT/'evaluation/build-cost/evidence/independent-candidate/prospective.json'


class CandidateTests(unittest.TestCase):
    def test_native_sdk_prefix_unchanged(self):
        receipt = json.loads(RECEIPT.read_text())
        dockerfile = (ROOT/'docker/native.Dockerfile').read_bytes()
        native = dockerfile.split(b'# Application compilation is cached independently')[0]
        self.assertEqual(hashlib.sha256(native).hexdigest(), receipt['unchanged_native_sdk_prefix_sha256'])

    def test_four_original_compile_commands_unchanged_except_reporting(self):
        receipt = json.loads(RECEIPT.read_text())
        text = (ROOT/'docker/native.Dockerfile').read_text()
        region = text.split('FROM native-build AS engine-build',1)[1].split('FROM scratch AS server',1)[0]
        commands = re.findall(r'cargo (?:test|build) --[^;]+', region)
        normalized = [re.sub(r' --message-format=json-render-diagnostics > /out/build-evidence/[A-Za-z0-9.-]+', '', command) for command in commands]
        self.assertEqual(len(normalized), 4)
        self.assertEqual(normalized, receipt['original_cargo_commands'])

    def test_selected_shell_runs_parse(self):
        text = (ROOT/'docker/native.Dockerfile').read_text()
        bodies = re.findall(r"RUN /bin/bash -euo pipefail -c '(.*?)'\n", text, re.S)
        self.assertGreaterEqual(len(bodies), 3)
        for body in bodies:
            done = subprocess.run(['bash', '-n'], input=body.replace('\\\n', ' '), text=True, capture_output=True)
            self.assertEqual(done.returncode, 0, done.stderr)

    def test_candidate_receipt_binds_exact_sources(self):
        receipt = json.loads(RECEIPT.read_text())
        directory = RECEIPT.parent
        source = json.loads((directory/'snapshot-source.json').read_text())
        self.assertRegex(source['source_commit'], r'^[0-9a-f]{40}$')
        self.assertEqual(source['source_path'], 'docker/native.Dockerfile')
        self.assertEqual(source['snapshot_file'], 'native.Dockerfile.snapshot')
        snapshot = (directory/source['snapshot_file']).read_bytes()
        self.assertEqual(hashlib.sha256(snapshot).hexdigest(), receipt['candidate_dockerfile_sha256'])
        self.assertEqual(source['sha256'], receipt['candidate_dockerfile_sha256'])
        git_blob = hashlib.sha1(b'blob ' + str(len(snapshot)).encode() + b'\0' + snapshot).hexdigest()
        self.assertEqual(git_blob, source['source_git_blob'])
        self.assertEqual(hashlib.sha256((ROOT/'scripts/select-cargo-artifact.py').read_bytes()).hexdigest(), receipt['selector_sha256'])


if __name__ == '__main__':
    unittest.main()
