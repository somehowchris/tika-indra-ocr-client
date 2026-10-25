"""Reviewed-lock admission with producer-shaped records, real OCI bytes and synthetic signatures.

The index, proof, cost and Chef key records come from the producers' own code.
No fixture authorizes an actual SDK, Chef layer, release or lock.
"""
import base64
import contextlib
import copy
import hashlib
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

ROOT=Path(__file__).resolve().parents[2]


def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


admit=load('lock_admit',ROOT/'scripts/admit-native-inputs.py')
cost=load('lock_cost_record',ROOT/'scripts/measure-native-build-cost.py')
renderer=load('lock_chef_renderer',ROOT/'scripts/render-chef-consumer.py')
sdk_test=load('lock_oci_fixture',ROOT/'evaluation/build-cost/test_sdk.py')
sdk=sdk_test.sdk
index_api=admit.index_api
REPOSITORY='somehowchris/tika-indra-ocr-client'
METADATA=('index.json','index.sigstore.json','sdk.spdx.json','chef.spdx.json','sdk-materialized/materialization.json','chef-oci-integrity.json',
          'chef-manifests/key.json','proof-result.json','cost-result.json','cost-protocol.json')


class LockTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.addCleanup(self.temporary.cleanup)
        self.base=Path(self.temporary.name).resolve();self.source=self.base/'checkout'
        for name in ('docker/native.Dockerfile','Cargo.toml','Cargo.lock','runtime/Cargo.toml','runtime/Cargo.lock',
                     'scripts/chef-artifact.py','scripts/render-chef-consumer.py','scripts/cargo-chef.lock.json',*sdk.INPUTS):
            destination=self.source/name;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/name,destination)
        self.git('init','--quiet');self.git('config','user.email','fixture@example.invalid');self.git('config','user.name','Synthetic fixture')
        self.git('add','.');self.git('commit','--quiet','-m','Synthetic producer source');producer_sha=self.git('rev-parse','HEAD')
        fixture=sdk_test.SdkTests();fixture.base=self.base
        with patch.object(sdk_test,'ROOT',self.source):self.layout,self.digest,_=fixture.fixture()
        self.index=self.produce(producer_sha)
        self.producer={name:self.index[name] for name in ('repository','source_sha','source_ref','release_run_id','release_run_attempt','workflow_ref','workflow_identity','oidc_issuer')}
        self.metadata=self.base/'lock/metadata'
        for name in METADATA:
            destination=self.metadata/name;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(self.base/'producer'/name,destination)
        self.lock={'schema_version':1,'kind':'admitted-native-build-inputs','mode':'chef','metadata_directory':'metadata','producer':self.producer,
                   'producer_artifact':{'id':321,'digest':'sha256:'+'a'*64},'index_sha256':index_api.sha(self.metadata/'index.json'),
                   'index_bundle_sha256':index_api.sha(self.metadata/'index.sigstore.json'),
                   'inputs':{role:entry['reference'] for role,entry in self.index['inputs'].items()},
                   'admission':{'proof_run_attempt':'1','proof_job_id':456,'proof_artifact_id':789,'proof_artifact_digest':'sha256:'+'e'*64,
                                'reviewed_net_cost':'Synthetic contract fixture only; no adoption.'}}
        self.rehash()
        (self.source/'consumer.txt').write_text('later consumer commit\n');self.git('add','.');self.git('commit','--quiet','-m','Synthetic consumer source')
        self.env={'GITHUB_REPOSITORY':REPOSITORY,'GITHUB_SHA':self.git('rev-parse','HEAD')}
        self.calls=[];self.real_run=subprocess.run

    def git(self,*args):return subprocess.check_output(['git',*args],cwd=self.source,text=True,stderr=subprocess.DEVNULL).strip()

    def produce(self,source_sha):
        """Evidence as publish-native-build-inputs.sh leaves it, then the producers' own records."""
        evidence=self.base/'producer';release=self.base/'release';evidence.mkdir()
        with contextlib.redirect_stdout(io.StringIO()):
            sdk.materialize(SimpleNamespace(layout=self.layout,digest=self.digest,output=evidence/'sdk-materialized',inputs=self.source))
        sdk.write(evidence/'chef-oci-integrity.json',sdk.verify_oci(self.layout,self.digest))
        (evidence/'chef-manifests').mkdir()
        for name in ('root','runtime'):(evidence/(name+'.json')).write_text('{"synthetic":"'+name+' recipe"}')
        with patch.dict(os.environ,{'SOURCE_DATE_EPOCH':'0'}):
            key=admit.chef_api.key(self.source,evidence/'root.json',evidence/'runtime.json',evidence/'sdk-materialized/sdk/recipe.json',
                                   evidence/'sdk-materialized/sdk/content.json',self.source/'scripts')
        admit.chef_api.write(evidence/'chef-manifests/key.json',key)
        admit.chef_api.write(evidence/'chef-manifests/content.json',{'schema_version':1,'dependency_key_sha256':key['dependency_key_sha256'],'entries':[]})
        for role in ('sdk','chef'):
            (evidence/(role+'.reference')).write_text('ghcr.io/somehowchris/tika-indra-native-'+role+'@'+self.digest+'\n')
            sdk.write(evidence/(role+'.spdx.json'),{'spdxVersion':'SPDX-2.3','name':'synthetic '+role})
        for role,(target,relative) in index_api.ROLES.items():
            path=evidence/'baseline'/target/relative;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'synthetic executable '+role.encode())
        (release/'bin').mkdir(parents=True);(release/'signing').mkdir()
        shutil.copyfile(evidence/'baseline/server-acceptance/indra-server',release/'bin/indra-server')
        ref='refs/tags/v-synthetic-producer';workflow=REPOSITORY+'/.github/workflows/release.yml@'+ref
        invocation={'GITHUB_REPOSITORY':REPOSITORY,'GITHUB_REF':ref,'GITHUB_SHA':source_sha,'GITHUB_WORKFLOW_REF':workflow,'GITHUB_RUN_ID':'123','GITHUB_RUN_ATTEMPT':'1'}
        sdk.write(release/'signing/candidates.json',{'repository':REPOSITORY,'source_sha':source_sha,'source_ref':ref,'release_run_id':'123','release_run_attempt':'1',
            'workflow_ref':workflow,'workflow_identity':'https://github.com/'+workflow,'oidc_issuer':'https://token.actions.githubusercontent.com',
            'images':[{'role':role} for role in ('backend','bridge','gateway','native','tesseract','tika')]})
        with patch.dict(os.environ,invocation,clear=True):index_api.create(self.source,release/'signing/candidates.json',evidence)
        index=index_api.read(evidence/'index.json')
        (evidence/'index.sigstore.json').write_text('{"fixture":"synthetic signature only"}')
        sdk.write(evidence/'proof-result.json',admit.proof_api.proof_result(index['source_sha']))
        protocol=cost.cost_protocol(index);sdk.write(evidence/'cost-protocol.json',protocol)
        records=[{**cell,'arm':arm,'whole_build_seconds':float(number)} for number,(cell,arm) in enumerate((cell,arm) for cell in protocol['schedule'] for arm in cell['order'])]
        sdk.write(evidence/'cost-result.json',cost.cost_result(protocol,records))
        return index

    def rehash(self):
        for name,key in (('proof-result.json','proof_result_sha256'),('cost-result.json','cost_result_sha256'),('cost-protocol.json','cost_protocol_sha256')):
            self.lock['admission'][key]=index_api.sha(self.metadata/name)
        self.lock_path=self.base/'lock/native-inputs.lock.json';self.lock_path.write_text(json.dumps(self.lock))

    def execute(self,command,**options):
        command=[str(v) for v in command]
        if command[0]=='git':return self.real_run(command,**options)
        self.calls.append(command);value={}
        def arg(name):return command[command.index(name)+1]
        if command[0]=='gh':
            job={'id':456,'run_id':123,'run_attempt':1,'head_sha':self.producer['source_sha'],'name':admit.PROOF_JOB,'status':'completed','conclusion':'success'}
            attempt={'id':123,'run_attempt':1,'head_sha':self.producer['source_sha'],'path':'.github/workflows/release.yml','event':'workflow_dispatch','status':'completed','conclusion':'success'}
            value=job if command[-1].endswith('/jobs/456') else attempt
        elif command[:2]==['cosign','verify-attestation']:
            role='chef' if '-native-chef@' in command[-1] else 'sdk'
            expected=self.index if arg('--type')==admit.PREDICATE else index_api.read(self.metadata/(role+'.spdx.json'))
            payload={'predicate':expected,'subject':[{'digest':{'sha256':self.digest.split(':')[1]}}]}
            value={'payloadType':'application/vnd.in-toto+json','payload':base64.b64encode(json.dumps(payload).encode()).decode()}
        elif command[0]=='crane':shutil.copytree(self.layout,Path(command[-1]))
        elif command[0]=='python3' and command[2]=='materialize-oci':
            with contextlib.redirect_stdout(io.StringIO()):sdk.materialize(SimpleNamespace(layout=Path(arg('--layout')),digest=arg('--digest'),inputs=Path(arg('--inputs')),output=Path(arg('--output'))))
        elif command[0]=='python3' and command[2]=='verify-oci':sdk.write(Path(arg('--output')),sdk.verify_oci(Path(arg('--layout')),arg('--digest')))
        elif command[0]=='python3' and command[1].endswith('render-chef-consumer.py'):renderer.render(Path(arg('--source')),Path(arg('--output')),arg('--mode'))
        elif command[0]=='python3' and command[2]=='render-consumer':sdk.render(SimpleNamespace(action='render-consumer',source=Path(arg('--source')),output=Path(arg('--output'))))
        elif command[0]!='cosign':raise AssertionError('unexpected command: '+repr(command))
        options['stdout'].write(json.dumps(value).encode())
        return subprocess.CompletedProcess(command,0)

    def admission(self,output,mode='locked'):
        arguments=['admit','--lock',str(self.lock_path),'--consumer-mode',mode,'--output',str(self.base/output)]
        with patch.object(admit,'ROOT',self.source),patch.dict(os.environ,self.env,clear=True),patch.object(sys,'argv',arguments),\
             patch.object(admit.subprocess,'run',side_effect=self.execute):admit.main()
        return index_api.read(self.base/output/'result.json')

    def test_reviewed_chef_lock_admits_the_producer_records(self):
        result=self.admission('admitted');chef=self.index['inputs']['chef'];sdk_input=self.index['inputs']['sdk']
        self.assertEqual((result['mode'],result['producer'],result['consumer_source_sha']),('chef',self.producer,self.env['GITHUB_SHA']))
        self.assertNotEqual(result['producer']['source_sha'],result['consumer_source_sha'])
        self.assertEqual(result['build_contexts'],['sdk-input=oci-layout://'+str(self.base/'admitted/sdk-oci')+'@'+sdk_input['digest'],
                                                  'chef-input=oci-layout://'+str(self.base/'admitted/chef-oci')+'@'+chef['digest']])
        self.assertEqual(result['build_args'],['BUILD_CPUSET=0,1','NATIVE_SDK_RECIPE_SHA256='+sdk_input['recipe_sha256'],'NATIVE_SDK_CONTENT_SHA256='+sdk_input['content_sha256'],
                                              'CHEF_DEPENDENCY_KEY_SHA256='+chef['recipe_sha256'],'CHEF_CONTENT_SHA256='+chef['content_sha256']])
        self.assertEqual(result['reviewed_admission'],self.lock['admission'])
        self.assertIn('FROM chef-restored AS engine-build',(self.source/admit.GENERATED).read_text())
        self.assertEqual([command[-1] for command in self.calls if command[0]=='crane'],[str(self.base/'admitted/sdk-oci'),str(self.base/'admitted/chef-oci')])
        self.assertFalse(any(command[0]=='docker' for command in self.calls))

    def test_changed_chef_inputs_refuse_chef_before_download_and_sdk_stays_admissible(self):
        for name in ('scripts/chef-artifact.py','runtime/Cargo.lock'):
            path=self.source/name;path.write_bytes(path.read_bytes()+b'\n# legitimate change\n');output='chef-refused-'+path.name
            with self.subTest(name=name):
                with self.assertRaisesRegex(ValueError,'explicitly select SDK-only compilation'):self.admission(output)
                self.assertFalse(any(command[0]=='crane' for command in self.calls))
                self.assertFalse(index_api.read(self.base/output/'completion.json')['source_fallback_attempted'])
            if name!='runtime/Cargo.lock':path.write_bytes(path.read_bytes().removesuffix(b'\n# legitimate change\n'))
        # A Rust-only lock change keeps the native SDK admissible without a republish.
        self.calls=[];result=self.admission('sdk-admitted','sdk')
        self.assertEqual(result['mode'],'sdk');self.assertEqual(len(result['build_contexts']),1)
        self.assertEqual([command[-1] for command in self.calls if command[0]=='crane'],[str(self.base/'sdk-admitted/sdk-oci')])

    def test_review_receipts_require_complete_current_producer_records(self):
        packet=self.metadata
        admit.reviewed_results(packet,self.lock,self.index)
        for name,change,message in (('proof-result.json',lambda v:v.update(roles_per_arm=7),'correctness proof required'),
                                    ('proof-result.json',lambda v:v.update(chef_invalidation_passed=False),'correctness proof required'),
                                    ('cost-result.json',lambda v:v.update(timed_builds=35),'paired cost result required'),
                                    ('cost-result.json',lambda v:v['paired_differences'].pop(),'paired cost result required'),
                                    ('cost-result.json',lambda v:v['summaries'].pop('material-doc'),'reviewed cost scenarios differ'),
                                    ('cost-result.json',lambda v:v['summaries']['warm'].pop('chef'),'reviewed cost arm set differs'),
                                    ('cost-result.json',lambda v:v['summaries']['warm']['sdk'].update(n=2),'three finite ordered cost repetitions required'),
                                    ('cost-result.json',lambda v:v['summaries']['source-comment']['chef'].update(min_seconds=-1.0),'three finite ordered cost repetitions required'),
                                    ('cost-result.json',lambda v:v['summaries']['material-doc']['sdk'].update(max_seconds=float('inf')),'three finite ordered cost repetitions required'),
                                    ('cost-result.json',lambda v:v['summaries']['warm']['chef'].update(min_seconds=v['summaries']['warm']['chef']['max_seconds']+1),'three finite ordered cost repetitions required'),
                                    ('cost-protocol.json',lambda v:v.update(targets=['engine-acceptance']),'different inputs/source')):
            original=(packet/name).read_bytes();value=json.loads(original);change(value);(packet/name).write_text(json.dumps(value))
            with self.subTest(name=name,message=message):
                with self.assertRaisesRegex(ValueError,'receipt differs'):admit.reviewed_results(packet,self.lock,self.index)
                self.rehash()
                with self.assertRaisesRegex(ValueError,message):admit.reviewed_results(packet,self.lock,self.index)
            (packet/name).write_bytes(original);self.rehash()
        admit.reviewed_results(packet,self.lock,self.index)

    def test_chef_key_binds_the_signed_sdk_and_the_application_epoch(self):
        packet=self.metadata;path=packet/'chef-manifests/key.json';original=path.read_bytes()
        admit.chef_compatibility(packet,self.index,self.source)
        for name in ('sdk_recipe_sha256','sdk_content_sha256'):
            key=json.loads(original);key['inputs'][name]='f'*64;path.write_text(json.dumps(key))
            with self.subTest(name=name):
                with self.assertRaisesRegex(ValueError,'signed Chef dependency key metadata differs'):admit.chef_compatibility(packet,self.index,self.source)
                # A producer whose signed Chef key was cooked against another SDK.
                key['dependency_key_sha256']=hashlib.sha256(admit.chef_api.canonical(key['inputs'])).hexdigest();path.write_text(json.dumps(key))
                index=copy.deepcopy(self.index);index['inputs']['chef']['recipe_sha256']=key['dependency_key_sha256']
                with self.assertRaisesRegex(ValueError,'Chef/native SDK compatibility boundary differs'):admit.chef_compatibility(packet,index,self.source)
            path.write_bytes(original)
        # A consumer that moves the pinned application epoch cannot reuse objects cooked under the old one.
        dockerfile=self.source/'docker/native.Dockerfile';text=dockerfile.read_text()
        self.assertIn('FROM native-build AS application-build\nENV SOURCE_DATE_EPOCH=0\n',text)
        dockerfile.write_text(text.replace('AS application-build\nENV SOURCE_DATE_EPOCH=0\n','AS application-build\nENV SOURCE_DATE_EPOCH=1\n'))
        with self.assertRaisesRegex(ValueError,'Chef application reproducibility environment differs'):admit.chef_compatibility(packet,self.index,self.source)

    def test_proof_attempt_cannot_precede_the_producer_attempt(self):
        admit.validate_lock(self.lock,REPOSITORY)
        early=copy.deepcopy(self.lock);early['producer']['release_run_attempt']='2'
        with self.assertRaisesRegex(ValueError,'precedes producer'):admit.validate_lock(early,REPOSITORY)


if __name__=='__main__':unittest.main()
