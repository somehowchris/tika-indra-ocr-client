"""Chef invalidation oracle: every mutation must be refused by its own gate. No Docker execution."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('chef_invalidation_control_test',ROOT/'scripts/verify-chef-invalidation.py')
controller=importlib.util.module_from_spec(spec);spec.loader.exec_module(controller)
COOKED_KEY='source/lock/features/profile/SDK cooked dependency identity differs'
SDK_ADMISSION='SDK admission manifest identity differs'
GATES={'source':COOKED_KEY,'lock':COOKED_KEY,'features':COOKED_KEY,'profile':COOKED_KEY,'sdk-content':SDK_ADMISSION}


class FixtureReached(Exception):
    """All five rejections were admitted; the native same-path fixture would build next."""


class ChefInvalidationTests(unittest.TestCase):
    def setUp(self):
        # Only the files the four source mutations edit, with their real bytes.
        buffer=io.BytesIO()
        with tarfile.open(fileobj=buffer,mode='w') as archive:
            for name in ('shared/trace-context/src/lib.rs','runtime/Cargo.lock','runtime/crates/indra-engine/Cargo.toml','runtime/Cargo.toml'):
                archive.add(ROOT/name,arcname=name)
        self.archive=buffer.getvalue()

    def verify(self,diagnostics):
        def execute(command,**options):
            command=[str(v) for v in command]
            if command[0]=='git':options['stdout'].write(self.archive);return subprocess.CompletedProcess(command,0)
            if command[0]=='python3':return subprocess.CompletedProcess(command,0)
            if command[command.index('--target')+1]!='chef-restored':raise FixtureReached
            case=Path(command[command.index('--file')+1]).name.removesuffix('.Dockerfile')
            status,body=diagnostics[case];options['stderr'].write(body.encode())
            return subprocess.CompletedProcess(command,status)
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve();inputs=root/'inputs';(inputs/'sdk-materialized').mkdir(parents=True)
            (inputs/'sdk-materialized/materialization.json').write_text(json.dumps({'recipe_sha256':'a'*64,'content_sha256':'b'*64}))
            index=root/'index.json'
            index.write_text(json.dumps({'inputs':{'sdk':{'digest':'sha256:'+'c'*64},'chef':{'digest':'sha256:'+'d'*64,'recipe_sha256':'e'*64,'content_sha256':'f'*64}}}))
            arguments=['verify-chef-invalidation','--index',str(index),'--inputs',str(inputs),'--output',str(root/'invalidation')]
            with patch.object(sys,'argv',arguments),patch.object(controller.subprocess,'run',side_effect=execute):controller.main()

    def test_each_case_is_admitted_only_by_its_intended_gate(self):
        with self.assertRaises(FixtureReached):self.verify({case:(1,gate) for case,gate in GATES.items()})
        wrong=[('features',(1,'source/lock/configuration changed during Chef prepare')),('profile',(1,SDK_ADMISSION)),
               ('sdk-content',(1,COOKED_KEY)),('sdk-content',(1,'SDK content identity differs')),('lock',(124,COOKED_KEY)),('source',(0,COOKED_KEY))]
        for case,outcome in wrong:
            with self.subTest(case=case,outcome=outcome),self.assertRaisesRegex(ValueError,'expected typed admission failure absent: reject-'+case+'$'):
                self.verify({**{name:(1,gate) for name,gate in GATES.items()},case:outcome})

    def test_every_accepted_diagnostic_is_raised_by_a_real_gate(self):
        raised_by={'chef-artifact.py':COOKED_KEY,'native-sdk.py':SDK_ADMISSION}
        self.assertEqual(set(controller.REJECTIONS.values()),set(raised_by.values()))
        for name,diagnostic in raised_by.items():self.assertIn("'"+diagnostic+"'",(ROOT/'scripts'/name).read_text())


if __name__=='__main__':unittest.main()
