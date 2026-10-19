"""Experimental selection contracts with real Git/OCI bytes and synthetic signatures.

No fixture authorizes an actual SDK, release, workflow dispatch or promotion.
"""
import base64
import copy
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import yaml

ROOT=Path(__file__).resolve().parents[2]


def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


admit=load('seed_admit',ROOT/'scripts/admit-native-inputs.py')
sdk_test=load('seed_oci_fixture',ROOT/'evaluation/build-cost/test_sdk.py')
sdk=sdk_test.sdk


class SeedTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.addCleanup(self.temporary.cleanup)
        self.base=Path(self.temporary.name).resolve();self.source=self.base/'checkout';self.source.mkdir()
        for name in ('docker/native.Dockerfile',*sdk.INPUTS):
            destination=self.source/name;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,destination)
        fixture=sdk_test.SdkTests();fixture.base=self.base
        with patch.object(sdk_test,'ROOT',self.source):self.layout,self.digest,_=fixture.fixture()
        with contextlib.redirect_stdout(io.StringIO()):sdk.materialize(SimpleNamespace(layout=self.layout,digest=self.digest,output=self.base/'initial',inputs=self.source))
        self.metadata=self.source/'proof/metadata';(self.metadata/'sdk-materialized').mkdir(parents=True)
        shutil.copyfile(self.base/'initial/materialization.json',self.metadata/'sdk-materialized/materialization.json')
        self.write(self.metadata/'sdk.spdx.json',{'spdxVersion':'SPDX-2.3','fixture':'not a real signed SDK'})
        self.write(self.metadata/'index.sigstore.json',{'fixture':'synthetic signature only'})
        repository=admit.SEED_REPOSITORY;ref='refs/tags/v-synthetic-prior';workflow=repository+'/.github/workflows/release.yml@'+ref
        producer={'repository':repository,'source_sha':'a'*40,'source_ref':ref,'release_run_id':'123','release_run_attempt':'2','workflow_ref':workflow,'workflow_identity':'https://github.com/'+workflow,'oidc_issuer':'https://token.actions.githubusercontent.com'}
        actual=admit.read(self.base/'initial/materialization.json')
        self.index={'schema_version':1,'kind':'native-build-inputs',**producer,'production_candidate_index_sha256':'c'*64,
            'inputs':{role:{'reference':'ghcr.io/somehowchris/tika-indra-native-'+role+'@'+self.digest,'digest':self.digest,'recipe_sha256':actual['recipe_sha256'],'content_sha256':actual['content_sha256'],'spdx_sha256':admit.index_api.sha(self.metadata/'sdk.spdx.json'),'oci_receipt_sha256':admit.index_api.sha(self.metadata/'sdk-materialized/materialization.json')} for role in ('sdk','chef')},
            'baseline_roles':{role:{'sha256':'d'*64,'bytes':1} for role in admit.index_api.ROLES}}
        self.write(self.metadata/'index.json',self.index)
        self.seed={'schema_version':1,'kind':'experimental-signed-native-sdk-seed','producer':producer,'metadata_directory':'metadata','index_sha256':admit.index_api.sha(self.metadata/'index.json'),'index_bundle_sha256':admit.index_api.sha(self.metadata/'index.sigstore.json'),'sdk_reference':self.index['inputs']['sdk']['reference'],'publish_job_id':456}
        self.seed_path=Path('proof/seed.json');self.write(self.source/self.seed_path,self.seed)
        self.git('init','--quiet');self.git('config','user.email','fixture@example.invalid');self.git('config','user.name','Synthetic fixture');self.commit()
        self.event=self.base/'event.json';self.event_value={'repository':{'full_name':repository},'inputs':{'native_input_mode':'sdk','run_sdk_proof':'true','proof_sdk_seed':self.seed_path.as_posix()}}
        self.write(self.event,self.event_value)
        self.env={'GITHUB_REPOSITORY':repository,'GITHUB_SHA':self.head,'GITHUB_WORKFLOW_SHA':self.head,'GITHUB_EVENT_NAME':'workflow_dispatch','GITHUB_REF':'refs/tags/v-synthetic-current','GITHUB_WORKFLOW_REF':repository+'/.github/workflows/release.yml@refs/tags/v-synthetic-current','GITHUB_JOB':'publish','GITHUB_EVENT_PATH':str(self.event)}
        self.attempt={'id':123,'run_attempt':2,'head_sha':producer['source_sha'],'head_branch':'v-synthetic-prior','head_repository':{'full_name':repository},'path':'.github/workflows/release.yml','event':'workflow_dispatch','status':'completed','conclusion':'failure'}
        self.job={'id':456,'run_id':123,'run_attempt':2,'head_sha':producer['source_sha'],'name':'publish','status':'completed','conclusion':'success'}
        self.jobs={'total_count':2,'jobs':[self.job,{'id':789,'run_id':123,'run_attempt':2,'head_sha':producer['source_sha'],'name':admit.PROOF_JOB,'status':'completed','conclusion':'failure'}]}
        self.calls=[];self.real_run=subprocess.run

    def write(self,path,value):path.write_text(json.dumps(value))
    def git(self,*args):return subprocess.check_output(['git',*args],cwd=self.source,text=True,stderr=subprocess.DEVNULL).strip()
    def commit(self):
        self.git('add','.');self.git('commit','--quiet','--allow-empty','-m','Synthetic seed contract fixture');self.head=self.git('rev-parse','HEAD')
        if hasattr(self,'env'):self.env.update(GITHUB_SHA=self.head,GITHUB_WORKFLOW_SHA=self.head)

    def execute(self,command,**options):
        command=[str(v) for v in command]
        if command[0]=='git':return self.real_run(command,**options)
        self.calls.append(command)
        output=options['stdout'];status=0;value={}
        try:
            if command[0]=='gh':
                route=command[-1]
                value=self.job if '/jobs/456' in route else self.jobs if '/jobs?' in route else self.attempt
            elif command[0]=='cosign':
                if command[1]=='verify-attestation':
                    expected=self.index if command[command.index('--type')+1]==admit.PREDICATE else admit.read(self.metadata/'sdk.spdx.json')
                    payload={'predicate':expected,'subject':[{'digest':{'sha256':self.digest.split(':')[1]}}]}
                    value={'payloadType':'application/vnd.in-toto+json','payload':base64.b64encode(json.dumps(payload).encode()).decode()}
            elif command[0]=='crane':shutil.copytree(self.layout,Path(command[-1]))
            elif command[0]=='python3' and command[2]=='materialize-oci':
                def arg(name):return Path(command[command.index(name)+1])
                with contextlib.redirect_stdout(io.StringIO()):sdk.materialize(SimpleNamespace(layout=arg('--layout'),digest=command[command.index('--digest')+1],inputs=arg('--inputs'),output=arg('--output')))
            elif command[0]=='python3' and command[2]=='render-consumer':
                sdk.render(SimpleNamespace(action='render-consumer',source=Path(command[command.index('--source')+1]),output=Path(command[command.index('--output')+1])))
            else:raise AssertionError('unexpected command: '+repr(command))
        except ValueError as error:
            options['stderr'].write(str(error).encode());status=1
        output.write(json.dumps(value).encode())
        return subprocess.CompletedProcess(command,status)

    def admission(self,output='admitted',execute=None):
        with patch.object(admit,'ROOT',self.source),patch.dict(os.environ,self.env,clear=True),patch.object(sys,'argv',['admit','--proof-sdk-seed',str(self.seed_path),'--consumer-mode','sdk','--output',str(self.base/output)]),patch.object(admit.subprocess,'run',side_effect=execute or self.execute):admit.main()
        return self.base/output

    def test_only_explicit_canonical_vtag_release_sdk_proof_context(self):
        with patch.object(admit,'ROOT',self.source),patch.dict(os.environ,self.env,clear=True):
            admit.seed_selection(self.seed_path,'sdk')
            for mode in ('locked','source','chef'):
                with self.assertRaisesRegex(ValueError,'explicit SDK-only'):admit.seed_selection(self.seed_path,mode)
            for key,value in [('GITHUB_EVENT_NAME','pull_request'),('GITHUB_EVENT_NAME','push'),('GITHUB_REPOSITORY','foreign/fork'),('GITHUB_REF','refs/heads/main'),('GITHUB_WORKFLOW_REF',self.env['GITHUB_WORKFLOW_REF'].replace('release.yml','native-service.yml')),('GITHUB_JOB','other'),('GITHUB_WORKFLOW_SHA','f'*40)]:
                with patch.dict(os.environ,{key:value}),self.assertRaises(ValueError):admit.seed_selection(self.seed_path,'sdk')
            for changes in ({'run_sdk_proof':'false'},{'native_input_mode':'locked'},{'proof_sdk_seed':'other.json'}):
                event=copy.deepcopy(self.event_value);event['inputs'].update(changes);self.write(self.event,event)
                with self.assertRaises(ValueError):admit.seed_selection(self.seed_path,'sdk')
            event=copy.deepcopy(self.event_value);event['repository']['full_name']='foreign/fork';self.write(self.event,event)
            with self.assertRaisesRegex(ValueError,'event repository'):admit.seed_selection(self.seed_path,'sdk')
        self.assertFalse(self.calls)

    def test_tracked_head_bytes_canonical_paths_and_bounded_seed_schema(self):
        with patch.object(admit,'ROOT',self.source),patch.dict(os.environ,self.env,clear=True):
            for bad in ('../seed.json','/tmp/seed.json','proof/../seed.json'):
                with self.assertRaises(ValueError):admit.seed_selection(Path(bad),'sdk')
            for name in ('proof/seed.json','proof/metadata/index.json','proof/metadata/sdk.spdx.json','proof/metadata/sdk-materialized/materialization.json'):
                path=self.source/name;original=path.read_bytes();path.write_bytes(original+b' ')
                with self.assertRaisesRegex(ValueError,'HEAD bytes'):admit.seed_selection(self.seed_path,'sdk')
                path.write_bytes(original)
            path=self.metadata/'sdk.spdx.json';original=path.read_bytes();path.unlink();path.symlink_to(self.metadata/'index.json')
            with self.assertRaisesRegex(ValueError,'canonical bounded'):admit.seed_selection(self.seed_path,'sdk')
            path.unlink();path.write_bytes(original)
            untracked=self.source/'proof/untracked.json';untracked.write_bytes((self.source/self.seed_path).read_bytes())
            with self.assertRaisesRegex(ValueError,'not present in committed HEAD'):admit.tracked_bytes(untracked)
        for key,value in [('publish_job_id',True),('sdk_reference','ghcr.io/somehowchris/tika-indra-native-sdk:latest'),('metadata_directory','../foreign'),('kind','admitted-native-build-inputs')]:
            changed=copy.deepcopy(self.seed);changed[key]=value
            with self.assertRaises(ValueError):admit.validate_seed(changed,admit.SEED_REPOSITORY)
        with self.assertRaises(ValueError):admit.validate_lock(self.seed,admit.SEED_REPOSITORY)

    def test_exact_completed_prior_publish_and_only_separate_proof_failure(self):
        status=admit.seed_producer(self.attempt,self.job,self.jobs,self.seed)
        self.assertEqual(status['run_conclusion'],'failure');self.assertFalse(status['routine_adoption_allowed'])
        for target,key,value in [('run','status','in_progress'),('run','head_branch','main'),('job','conclusion','failure'),('job','name','other')]:
            run=copy.deepcopy(self.attempt);job=copy.deepcopy(self.job);jobs=copy.deepcopy(self.jobs)
            (run if target=='run' else job)[key]=value
            with self.assertRaises(ValueError):admit.seed_producer(run,job,jobs,self.seed)
        for mutate in (lambda jobs:jobs['jobs'][1].update(name='unrelated'),lambda jobs:jobs.update(total_count=101),lambda jobs:jobs['jobs'][1].update(run_attempt=3)):
            jobs=copy.deepcopy(self.jobs);mutate(jobs)
            with self.assertRaises(ValueError):admit.seed_producer(self.attempt,self.job,jobs,self.seed)

    def test_real_oci_admission_remains_unpromoted_and_retains_parent_provenance(self):
        output=self.admission();result=admit.read(output/'result.json')
        self.assertTrue(result['experimental_seed']);self.assertFalse(result['routine_adoption_allowed']);self.assertIsNone(result['reviewed_admission'])
        self.assertEqual(result['producer']['source_sha'],'a'*40);self.assertEqual(result['consumer_source_sha'],self.head)
        self.assertEqual(result['parent_sdk']['reference'],self.seed['sdk_reference'])
        self.assertEqual(result['seed_producer_status']['failed_jobs'][0]['name'],admit.PROOF_JOB)
        sdk=self.index['inputs']['sdk'];producer=self.seed['producer'];reference=sdk['reference']
        policy=['--certificate-identity',producer['workflow_identity'],'--certificate-oidc-issuer',producer['oidc_issuer'],
            '--certificate-github-workflow-repository',producer['repository'],'--certificate-github-workflow-ref',producer['source_ref'],'--certificate-github-workflow-sha',producer['source_sha']]
        self.assertEqual([command for command in self.calls if command[0]=='crane'],
            [['crane','pull','--format=oci',reference,str(output/'sdk-oci')]])
        self.assertEqual([command for command in self.calls if command[:2]==['cosign','verify']],
            [['cosign','verify',*policy,reference]])
        self.assertEqual([command for command in self.calls if command[:2]==['cosign','verify-attestation']],
            [['cosign','verify-attestation',*policy,'--type',kind,reference] for kind in ('spdxjson',admit.PREDICATE)])
        self.assertEqual([command[2] for command in self.calls if command[0]=='python3'],['materialize-oci','render-consumer'])
        self.assertEqual(result['mode'],'sdk')
        self.assertEqual(result['build_contexts'],['sdk-input=oci-layout://'+str(output/'sdk-oci')+'@'+sdk['digest']])
        self.assertEqual(result['build_args'],['BUILD_CPUSET=0,1','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256']])
        self.assertFalse(any(command[0]=='docker' for command in self.calls))
        self.assertEqual(admit.read(output/'sdk-materialized/materialization.json')['content_sha256'],self.index['inputs']['sdk']['content_sha256'])
        with patch.object(admit,'ROOT',self.source),patch.dict(os.environ,self.env,clear=True):admit.prepare_seed_producer(self.seed_path,output,self.base/'parent-sdk')
        parent=admit.read(self.base/'parent-sdk/provenance.json');self.assertEqual(parent['producer'],self.seed['producer']);self.assertFalse(parent['fresh_proof_pass_claim'])
        self.assertEqual((self.base/'parent-sdk/metadata/index.sigstore.json').read_bytes(),(self.metadata/'index.sigstore.json').read_bytes())
        inventory=admit.index_api.parent_files(self.base);index={'parent_sdk':{'files':inventory,'provenance':parent}}
        admit.index_api.verify_parent(index,self.base)
        (self.base/'parent-sdk/current-materialization.json').write_text('{}')
        with self.assertRaisesRegex(ValueError,'inventory differs'):admit.index_api.verify_parent(index,self.base)

    def test_signature_failure_stops_before_pull_and_never_source_fallback(self):
        def failed(command,**options):
            if command[0]=='cosign':self.calls.append([str(v) for v in command]);return subprocess.CompletedProcess(command,42)
            return self.execute(command,**options)
        with self.assertRaisesRegex(ValueError,'index-signature'):self.admission(execute=failed)
        self.assertFalse(any(command[0] in ('crane','docker') for command in self.calls))
        completion=admit.read(self.base/'admitted/completion.json');self.assertFalse(completion['passed']);self.assertFalse(completion['source_fallback_attempted'])

    def test_corrupted_actual_oci_bytes_fail_same_reader_without_fallback(self):
        def corrupt(command,**options):
            result=self.execute(command,**options)
            if command[0]=='crane':
                blob=next((Path(command[-1])/'blobs/sha256').iterdir());blob.write_bytes(blob.read_bytes()[:-1])
            return result
        with self.assertRaisesRegex(ValueError,'sdk-materialize'):self.admission(execute=corrupt)
        self.assertFalse((self.base/'admitted/result.json').exists());self.assertFalse(admit.read(self.base/'admitted/completion.json')['source_fallback_attempted'])

    def test_current_native_recipe_change_and_mismatched_attestation_hard_fail(self):
        source=self.source/'docker/native.Dockerfile';original=source.read_bytes();source.write_bytes(b'# changed native recipe\n'+original)
        with self.assertRaisesRegex(ValueError,'sdk-materialize'):self.admission('changed-source')
        source.write_bytes(original);self.calls=[]
        def wrong_predicate(command,**options):
            if command[0]=='cosign' and command[1]=='verify-attestation':
                self.calls.append([str(v) for v in command]);options['stdout'].write(b'[]');return subprocess.CompletedProcess(command,0)
            return self.execute(command,**options)
        with self.assertRaisesRegex(ValueError,'attestation'):self.admission('wrong-predicate',wrong_predicate)
        self.assertFalse(any(command[0] in ('crane','docker') for command in self.calls))

    def test_producer_consumer_render_has_no_native_compile_and_current_provenance(self):
        producer=self.base/'producer.Dockerfile';consumer=self.base/'consumer.Dockerfile'
        sdk.render(SimpleNamespace(action='render-producer',source=ROOT/'docker/native.Dockerfile',output=producer))
        sdk.render(SimpleNamespace(action='render-consumer',source=producer,output=consumer))
        text=consumer.read_text();self.assertNotIn('bash scripts/build-native-runtime.sh /native',text)
        self.assertIn('FROM sdk-input AS native-sdk-input',text);self.assertIn('--source-commit "$SOURCE_COMMIT"',text)
        self.assertEqual(text.split('FROM native-build AS native-sdk-package',1)[1],producer.read_text().split('FROM native-build AS native-sdk-package',1)[1])
        workflow=yaml.safe_load((ROOT/'.github/workflows/release.yml').read_text());steps=workflow['jobs']['publish']['steps']
        early=next(i for i,v in enumerate(steps) if v.get('name','').startswith('Reject experimental seed'))
        self.assertLess(early,next(i for i,v in enumerate(steps) if 'docker/login-action@' in v.get('uses','')))
        action=yaml.safe_load((ROOT/'.github/actions/native-inputs/action.yml').read_text());selected=action['runs']['steps'][0]['run']
        self.assertIn('--validate-selection-only',selected)
        script=(ROOT/'scripts/publish-native-build-inputs.sh').read_text()
        self.assertIn('--file "$baseline_recipe"',script);self.assertIn('"${sdk_inputs[@]}"',script)
        self.assertIn('docker/native-inputs.generated.Dockerfile',(ROOT/'.dockerignore').read_text().splitlines())


if __name__=='__main__':unittest.main()
