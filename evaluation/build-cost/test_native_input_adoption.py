"""Consumer trust/scope contracts. Synthetic fixtures never admit real inputs."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import yaml

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('admission_test',ROOT/'scripts/admit-native-inputs.py');admit=importlib.util.module_from_spec(spec);spec.loader.exec_module(admit)


def fixture():
    repository='somehowchris/tika-indra-ocr-client';ref='refs/tags/v-fixture';workflow=repository+'/.github/workflows/release.yml@'+ref
    return {'schema_version':1,'kind':'admitted-native-build-inputs','mode':'sdk','metadata_directory':'native-inputs',
        'producer':{'repository':repository,'source_sha':'a'*40,'source_ref':ref,'release_run_id':'123','release_run_attempt':'1','workflow_ref':workflow,'workflow_identity':'https://github.com/'+workflow,'oidc_issuer':'https://token.actions.githubusercontent.com'},
        'producer_artifact':{'id':123,'digest':'sha256:'+'a'*64},'index_sha256':'b'*64,'index_bundle_sha256':'c'*64,
        'inputs':{role:'ghcr.io/somehowchris/tika-indra-native-'+role+'@sha256:'+'d'*64 for role in ('sdk','chef')},
        'admission':{'proof_run_attempt':'1','proof_job_id':456,'proof_artifact_id':789,'proof_artifact_digest':'sha256:'+'e'*64,'proof_result_sha256':'f'*64,'cost_result_sha256':'1'*64,'cost_protocol_sha256':'2'*64,'reviewed_net_cost':'Synthetic contract fixture only; no adoption.'}}


class AdoptionTests(unittest.TestCase):
    def test_public_metadata_copy_refuses_symlinked_ancestors_before_reading(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';source.mkdir();packet=root/'packet';packet.mkdir();outside=root/'outside';outside.mkdir()
            (outside/'receipt.json').write_text('{"private":"must not be copied"}')
            (source/'nested').symlink_to(outside,target_is_directory=True)
            with self.assertRaisesRegex(ValueError,'crosses a symlink'):admit.copy_metadata(source,packet,['nested/receipt.json'])
            self.assertFalse((packet/'nested').exists())

    def test_producer_identity_remains_distinct_from_current_consumer_source(self):
        lock=fixture()
        with patch.dict(os.environ,{'GITHUB_SHA':'c'*40,'GITHUB_REF':'refs/pull/7/merge'}):
            producer=admit.validate_lock(lock,lock['producer']['repository'])
        self.assertEqual(producer['source_sha'],'a'*40)
        self.assertEqual(producer['source_ref'],'refs/tags/v-fixture')
        for section,key,bad in [('producer','source_ref','refs/heads/untrusted'),('producer','workflow_ref','other/release.yml@refs/tags/v-fixture'),('producer','oidc_issuer','https://example.invalid'),('inputs','sdk','ghcr.io/somehowchris/tika-indra-native-sdk:latest'),('admission','proof_run_attempt','0'),('admission','reviewed_net_cost','')]:
            value=copy.deepcopy(lock);value[section][key]=bad
            with self.assertRaises(ValueError):admit.validate_lock(value,lock['producer']['repository'])
        with self.assertRaises(ValueError):admit.validate_lock(lock,'other/repository')
        for key,bad in [('schema_version',True),('index_sha256','missing'),('metadata_directory','../foreign'),('mode','source')]:
            value=copy.deepcopy(lock);value[key]=bad
            with self.assertRaises(ValueError):admit.validate_lock(value,lock['producer']['repository'])

    def test_actual_complete_result_contract_rejects_partial_changed_and_nonfinite_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);lock=fixture();index={'source_sha':'a'*40,'inputs':{'sdk':{'identity':'sdk'},'chef':{'identity':'chef'}}}
            proof={'passed':True,'source_sha':index['source_sha'],'arms':['sdk','chef'],'roles_per_arm':7,'cpu_caps':[1,2],'source_materials_equal':True,'chef_invalidation_passed':True,'production_candidate_images':6}
            cost={'passed':True,'timed_builds':36,'paired_differences':[{}]*9,'summaries':{scenario:{arm:{'n':3,'min_seconds':1,'median_seconds':2,'max_seconds':3} for arm in ('sdk','chef')} for scenario in ('warm','source-comment','material-doc')}}
            protocol={'source_sha':index['source_sha'],'inputs':index['inputs'],'targets':['engine-acceptance','server-acceptance']}
            def retain():
                for name,key,value in [('proof-result.json','proof_result_sha256',proof),('cost-result.json','cost_result_sha256',cost),('cost-protocol.json','cost_protocol_sha256',protocol)]:
                    path=root/name;path.write_text(json.dumps(value));lock['admission'][key]=hashlib.sha256(path.read_bytes()).hexdigest()
            retain();admit.reviewed_results(root,lock,index)
            proof['chef_invalidation_passed']=False;retain()
            with self.assertRaises(ValueError):admit.reviewed_results(root,lock,index)
            proof['chef_invalidation_passed']=True;protocol['source_sha']='b'*40;retain()
            with self.assertRaises(ValueError):admit.reviewed_results(root,lock,index)
            protocol['source_sha']='a'*40;cost['summaries']['warm']['chef']['median_seconds']=float('nan');retain()
            with self.assertRaises(ValueError):admit.reviewed_results(root,lock,index)
            cost['summaries']['warm']['chef']['median_seconds']=2;retain();(root/'proof-result.json').write_text('{}')
            with self.assertRaises(ValueError):admit.reviewed_results(root,lock,index)

    def test_real_source_key_change_requires_explicit_sdk_before_chef_restore(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';packet=root/'metadata';(packet/'chef-manifests').mkdir(parents=True)
            for name in ('Cargo.toml','Cargo.lock','runtime/Cargo.toml','runtime/Cargo.lock','shared/example/src/lib.rs','scripts/chef-artifact.py','scripts/render-chef-consumer.py','scripts/cargo-chef.lock.json'):
                path=source/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(name)
            index={'inputs':{'sdk':{'recipe_sha256':'a'*64,'content_sha256':'b'*64},'chef':{}}}
            inputs={'workspace_inputs':admit.chef_api.source_inputs(source),'build_tools':{name:admit.index_api.sha(source/'scripts'/name) for name in ('chef-artifact.py','render-chef-consumer.py','cargo-chef.lock.json')},'sdk_recipe_sha256':'a'*64,'sdk_content_sha256':'b'*64}
            key=hashlib.sha256(admit.chef_api.canonical(inputs)).hexdigest();index['inputs']['chef']['recipe_sha256']=key
            (packet/'chef-manifests/key.json').write_text(json.dumps({'dependency_key_sha256':key,'inputs':inputs}))
            admit.chef_compatibility(packet,index,source)
            (source/'runtime/Cargo.lock').write_text('legitimate changed Rust dependency lock')
            with self.assertRaisesRegex(ValueError,'explicitly select SDK-only'):admit.chef_compatibility(packet,index,source)
            # Native SDK identities remain the same; this is not a corrupted
            # input and does not require producing a new native SDK.
            self.assertEqual(index['inputs']['sdk']['content_sha256'],'b'*64)

    def test_supplied_lock_admission_failure_does_not_execute_a_source_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);tools=root/'bin';tools.mkdir();calls=root/'calls';lock=root/'lock.json';lock.write_text(json.dumps(fixture()))
            for name in ('gh','cosign','crane','docker'):
                path=tools/name;path.write_text('#!/bin/sh\nprintf "%s\\n" "$0 $*" >> "$ADMISSION_CALLS"\nexit 42\n');path.chmod(0o755)
            env={**os.environ,'PATH':str(tools)+os.pathsep+os.environ['PATH'],'ADMISSION_CALLS':str(calls),'GITHUB_REPOSITORY':fixture()['producer']['repository'],'GITHUB_SHA':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()}
            result=subprocess.run(['python3','-B',ROOT/'scripts/admit-native-inputs.py','--lock',lock,'--output',root/'output'],cwd=ROOT,env=env,capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('gh api',calls.read_text());self.assertNotIn('docker',calls.read_text());self.assertNotIn('crane',calls.read_text())
            completion=json.loads((root/'output/completion.json').read_text());self.assertFalse(completion['passed']);self.assertFalse(completion['source_fallback_attempted'])

    def test_sdk_profile_export_retains_its_original_diagnostic_stages(self):
        spec=importlib.util.spec_from_file_location('adoption_sdk_render',ROOT/'scripts/native-sdk.py');sdk=importlib.util.module_from_spec(spec);spec.loader.exec_module(sdk)
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'consumer.Dockerfile';source=ROOT/'docker/native.Dockerfile'
            sdk.render(SimpleNamespace(action='render-consumer',source=source,output=output))
            original=source.read_text().split('FROM native-build AS engine-profile-build',1)[1]
            restored=output.read_text().split('FROM native-build AS engine-profile-build',1)[1]
            self.assertEqual(restored,original,'profile jq/tools, original compilation and complete export must survive SDK rendering')
            self.assertIn('apt-get install -y --no-install-recommends jq',restored)
            self.assertIn('AS engine-profile-export',restored)
            self.assertNotIn('FROM chef-restored AS engine-profile-build',output.read_text())

    def test_missing_lock_selects_source_before_verification_and_profile_selects_sdk(self):
        action=yaml.safe_load((ROOT/'.github/actions/native-inputs/action.yml').read_text());prepare=next(step['run'] for step in action['runs']['steps'] if step.get('id')=='prepare')
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);output=root/'outputs';output.touch()
            result=subprocess.run(['bash','-euo','pipefail','-c',prepare],cwd=ROOT,env={**os.environ,'RUNNER_TEMP':str(root),'GITHUB_OUTPUT':str(output),'GITHUB_SHA':'c'*40,'PINNED':'false','CONSUMER_MODE':'locked'},capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertIn('mode=source\ndockerfile=docker/native.Dockerfile\n',output.read_text())
            self.assertEqual(len(list(root.rglob('source.json'))),1);self.assertFalse(list(root.rglob('admitted')))
        workflow=yaml.safe_load((ROOT/'.github/workflows/native-profile.yml').read_text());steps=workflow['jobs']['profile']['steps'];selected=next(step for step in steps if step.get('id')=='native_inputs')
        self.assertEqual(selected['with']['consumer-mode'],"${{ inputs.native_input_mode || 'sdk' }}")


if __name__=='__main__':unittest.main()
