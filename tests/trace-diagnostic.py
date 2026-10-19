#!/usr/bin/env python3
"""Offline admission tests; these never run Docker, downloads, builds or OCR."""
from __future__ import annotations
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('trace_diagnostic',ROOT/'scripts/prepare-trace-diagnostic.py')
module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
FIXTURE = json.loads((ROOT/'tests/fixtures/trace-diagnostic-source41.json').read_text())


class Admission(unittest.TestCase):
    def test_actual_completed_api_projection(self):
        module.validate_api(FIXTURE['run'],FIXTURE['job'],FIXTURE['artifact'])

    def test_wrong_api_identity_or_unrelated_failure_rejected(self):
        mutations = [('run','head_sha','0'*40),('run','event','push'),('run','run_attempt',2),('run','conclusion','success'),('artifact','digest','sha256:'+'0'*64),('artifact','size_in_bytes',module.ARTIFACT_BYTES+1),('artifact','expired',True),('job','head_sha','0'*40)]
        for section,key,value in mutations:
            with self.subTest(section=section,key=key):
                fixture = copy.deepcopy(FIXTURE); fixture[section][key] = value
                with self.assertRaises(ValueError):
                    module.validate_api(fixture['run'],fixture['job'],fixture['artifact'])
        for kind in ('second_failure','wrong_failure','missing_prerequisite'):
            with self.subTest(kind=kind):
                fixture = copy.deepcopy(FIXTURE)
                if kind == 'second_failure':
                    fixture['job']['steps'].append({'name':'unrelated failure','conclusion':'failure'})
                elif kind == 'wrong_failure':
                    next(s for s in fixture['job']['steps'] if s['conclusion']=='failure')['name'] = 'unrelated failure'
                else:
                    next(s for s in fixture['job']['steps'] if s['name']=='Run real native and gateway process gates at 1 and 2 CPU')['conclusion'] = 'skipped'
                with self.assertRaises(ValueError):
                    module.validate_api(fixture['run'],fixture['job'],fixture['artifact'])

    def test_foreign_invocations_fail_before_git_or_network(self):
        env = {'GITHUB_REPOSITORY':module.REPOSITORY,'GITHUB_EVENT_NAME':'push','GITHUB_REF':module.REF,'GITHUB_JOB':'replay','GITHUB_WORKFLOW_REF':module.REPOSITORY+'/.github/workflows/trace-diagnostic.yml@'+module.REF,'GITHUB_SHA':'1'*40}
        for key,value in [('GITHUB_REPOSITORY','foreign/repo'),('GITHUB_REF','refs/heads/main'),('GITHUB_EVENT_NAME','workflow_dispatch'),('GITHUB_JOB','publish'),('GITHUB_WORKFLOW_REF','foreign/workflow')]:
            with self.subTest(key=key), patch.dict(os.environ,{**env,key:value},clear=True), patch.object(module,'git') as git:
                with self.assertRaises(ValueError):
                    module.context()
                git.assert_not_called()

    def test_only_exact_reviewed_packaging_delta_is_admitted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); producer = root/'producer'; controller = root/'controller'
            producer.mkdir(); controller.mkdir()
            for name in module.UNCHANGED:
                for base in (producer,controller):
                    path = base/name; path.parent.mkdir(parents=True,exist_ok=True); path.write_text('unchanged '+name)
            recipe = 'docker/trace-acceptance.Dockerfile'; helper = 'scripts/prepare-trace-helper.sh'
            (producer/recipe).write_text('old exact recipe')
            (controller/recipe).write_text('new exact named-context recipe')
            (controller/helper).write_text('new exact helper')
            reviewed = {recipe:{'producer_sha256':module.sha(producer/recipe),'controller_sha256':module.sha(controller/recipe)},helper:{'producer_absent':True,'controller_sha256':module.sha(controller/helper)}}
            def git(source,*args):
                return module.PRODUCER.encode()+b'\n' if args == ('rev-parse','HEAD') else b''
            with patch.object(module,'ROOT',controller),patch.object(module,'REVIEWED',reviewed),patch.object(module,'git',git):
                self.assertEqual(len(module.closure(producer)),len(module.UNCHANGED)+2)
                gate = controller/module.UNCHANGED[0]; original = gate.read_bytes(); gate.write_text('changed gate')
                with self.assertRaises(ValueError): module.closure(producer)
                gate.write_bytes(original)
                (controller/recipe).write_text('unreviewed recipe')
                with self.assertRaises(ValueError): module.closure(producer)
                (controller/recipe).write_text('new exact named-context recipe')
                (producer/helper).write_text('unexpected old helper')
                with self.assertRaises(ValueError): module.closure(producer)

    def archive(self,root,kind='valid'):
        names = ['source-files.sha256','prepare-result.json','process/result.json','references/result.json','acceptance-build/gateway/binaries.sha256','materials/gateway/rust-source.tar.gz','materials/gateway/SHA256SUMS','materials/tika/java/tika-server.jar','bin/indra-server','bin/indra-gateway','bin/indra-tesseract',*('build/'+role+'/image.json' for role in module.ROLES)]
        archive = root/'original.zip'
        with warnings.catch_warnings(), zipfile.ZipFile(archive,'w') as output:
            warnings.simplefilter('ignore',UserWarning)
            for name in names:
                output.writestr(name,b'synthetic archive-path fixture only')
            if kind == 'duplicate': output.writestr(names[0],b'duplicate')
            if kind == 'traversal': output.writestr('../escape',b'escape')
            if kind == 'symlink':
                info = zipfile.ZipInfo('link'); info.external_attr = (stat.S_IFLNK|0o777)<<16; output.writestr(info,b'target')
        return archive

    def test_safe_exclusive_extraction_and_full_hash_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); archive = self.archive(root)
            with patch.object(module,'ARTIFACT_BYTES',archive.stat().st_size), patch.object(module,'ARTIFACT_SHA',module.sha(archive)):
                module.unpack(archive,root/'output')
                self.assertTrue((root/'output/bin/indra-server').is_file())
                with self.assertRaises(ValueError): module.unpack(archive,root/'output')
                with patch.object(module,'ARTIFACT_SHA','0'*64):
                    with self.assertRaises(ValueError): module.unpack(archive,root/'wrong-hash')

    def test_rehashed_unsafe_archives_rejected(self):
        for kind in ('duplicate','traversal','symlink'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp); archive = self.archive(root,kind)
                # Rehash every mutant so the path/type guard, not checksum mismatch, is exercised.
                with patch.object(module,'ARTIFACT_BYTES',archive.stat().st_size), patch.object(module,'ARTIFACT_SHA',module.sha(archive)):
                    with self.assertRaises(ValueError): module.unpack(archive,root/'output')
                self.assertFalse((root/'escape').exists())


if __name__ == '__main__':
    unittest.main()
