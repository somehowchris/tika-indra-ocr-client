"""Cheap diagnostic admission negatives; no registry, Kubernetes or inference calls."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import yaml

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('kubernetes_replay_test',ROOT/'scripts/prepare-kubernetes-replay.py');replay=importlib.util.module_from_spec(spec);spec.loader.exec_module(replay)
PLAN=json.loads((ROOT/'evaluation/kubernetes/replay-39213042394.json').read_text())


class KubernetesReplayTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name).resolve()

    def test_fixed_context_rejects_other_events_repositories_refs_and_workflows(self):
        env={'GITHUB_REPOSITORY':PLAN['repository'],'GITHUB_EVENT_NAME':'push','GITHUB_REF':replay.REF,'GITHUB_JOB':'replay','GITHUB_WORKFLOW_REF':PLAN['repository']+'/.github/workflows/kubernetes-release-replay.yml@'+replay.REF,'GITHUB_SHA':'a'*40}
        with patch.dict(os.environ,env,clear=True),patch.object(replay,'git',return_value=b'a'*40),patch.object(replay.subprocess,'run',return_value=subprocess.CompletedProcess([],0)):
            replay.context(PLAN)
            for key,value in [('GITHUB_EVENT_NAME','pull_request'),('GITHUB_EVENT_NAME','workflow_dispatch'),('GITHUB_REPOSITORY','foreign/fork'),('GITHUB_REF','refs/heads/main'),('GITHUB_JOB','publish'),('GITHUB_SHA','b'*40),('GITHUB_WORKFLOW_REF',env['GITHUB_WORKFLOW_REF'].replace('kubernetes-release-replay','release'))]:
                with patch.dict(os.environ,{key:value}),self.assertRaises(ValueError):replay.context(PLAN)

    def test_original_failed_run_job_and_complete_artifact_are_exact(self):
        run={'id':PLAN['run_id'],'run_attempt':1,'head_sha':PLAN['producer_source_sha'],'head_branch':PLAN['producer_ref'].removeprefix('refs/tags/'),'path':'.github/workflows/release.yml','event':'workflow_dispatch','status':'completed','conclusion':'failure','repository':{'full_name':PLAN['repository']},'head_repository':{'full_name':PLAN['repository']}}
        job={'id':PLAN['publish_job_id'],'run_id':PLAN['run_id'],'run_attempt':1,'head_sha':PLAN['producer_source_sha'],'name':'publish','status':'completed','conclusion':'failure','steps':[{'name':'Finish accepted native OCR during real Kubernetes sidecar shutdown','conclusion':'failure'}]}
        artifact={'id':PLAN['artifact_id'],'name':PLAN['artifact_name'],'size_in_bytes':PLAN['artifact_bytes'],'digest':'sha256:'+PLAN['artifact_sha256'],'expired':False,'workflow_run':{'id':PLAN['run_id'],'head_sha':PLAN['producer_source_sha']}}
        replay.validate_api(run,job,artifact,PLAN)
        for section,key,value in [('run','head_sha','b'*40),('run','conclusion','success'),('job','id',1),('job','conclusion','success'),('artifact','size_in_bytes',1),('artifact','digest','sha256:'+'b'*64),('artifact','expired',True)]:
            values={'run':copy.deepcopy(run),'job':copy.deepcopy(job),'artifact':copy.deepcopy(artifact)};values[section][key]=value
            with self.assertRaises(ValueError):replay.validate_api(values['run'],values['job'],values['artifact'],PLAN)
        job['steps'].append({'name':'unrelated failure','conclusion':'failure'})
        with self.assertRaises(ValueError):replay.validate_api(run,job,artifact,PLAN)

    def test_whole_zip_digest_and_rehashed_unsafe_member_fail(self):
        archive=self.root/'source.zip'
        with zipfile.ZipFile(archive,'w') as stream:stream.writestr('valid','fixture')
        size=archive.stat().st_size;digest=replay.sha(archive)
        replay.unpack(archive,self.root/'valid',size,digest,['valid'])
        with self.assertRaisesRegex(ValueError,'length/SHA'):replay.unpack(archive,self.root/'wrong-size',size+1,digest,['valid'])
        with self.assertRaisesRegex(ValueError,'length/SHA'):replay.unpack(archive,self.root/'wrong-hash',size,'0'*64,['valid'])
        with self.assertRaisesRegex(ValueError,'missing'):replay.unpack(archive,self.root/'missing',size,digest,['absent'])
        with zipfile.ZipFile(archive,'w') as stream:stream.writestr('../escape','fixture')
        with self.assertRaisesRegex(ValueError,'unsafe'):replay.unpack(archive,self.root/'unsafe',archive.stat().st_size,replay.sha(archive),['../escape'])

    def test_controller_and_deployment_closure_have_only_exact_recorder_exception(self):
        producer=self.root/'producer';controller=self.root/'controller'
        for root in (producer,controller):
            for name in (*replay.HELPERS,'deploy/base/kustomization.yaml','scripts/record-cri-version.sh'):
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(name)
        recorder=replay.sha(controller/'scripts/record-cri-version.sh')
        with patch.object(replay,'git',return_value=b'deploy/base/kustomization.yaml\0'):
            replay.controller_closure(producer,controller,recorder)
            for name in ('deploy/base/kustomization.yaml','scripts/verify-kubernetes-native.sh','scripts/record-cri-version.sh'):
                path=controller/name;old=path.read_text();path.write_text('unexpected source change')
                with self.assertRaises(ValueError):replay.controller_closure(producer,controller,recorder)
                path.write_text(old)

    def test_original_harness_source_and_reference_inner_checks_are_not_vacuous(self):
        producer=self.root/'producer';packet=self.root/'packet';producer.mkdir();packet.mkdir();plan=copy.deepcopy(PLAN)
        sources={f'runtime/src/file{i}.rs':b'// inert synthetic source fixture\n' for i in range(100)}
        for name,body in sources.items():path=producer/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
        for command in (['git','init','--quiet'],['git','add','.'],['git','-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','--quiet','-m','fixture']):subprocess.run(command,cwd=producer,check=True)
        plan['producer_source_sha']=subprocess.check_output(['git','rev-parse','HEAD'],cwd=producer,text=True).strip()
        def retain(name,body):
            path=packet/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
        harness=b'inert synthetic executable identity fixture; never executed';plan['harness_sha256']=hashlib.sha256(harness).hexdigest()
        retain('bin/indra-stack-acceptance',harness);retain('bin/indra-stack-acceptance.sha256',(plan['harness_sha256']+'  /original/path\n').encode());retain('acceptance-build/gateway/binaries.sha256',(plan['harness_sha256']+'  /out/indra-stack-acceptance\n').encode())
        for role,value in plan['images'].items():retain('build/'+role+'/image.json',json.dumps(value).encode())
        retain('prepare-result.json',b'{"passed":true}');retain('source-files.sha256',''.join(hashlib.sha256(body).hexdigest()+'  '+name+'\n' for name,body in sources.items()).encode())
        def bundle(mutated=False):
            path=packet/'materials/gateway/rust-source.tar.gz';path.parent.mkdir(parents=True,exist_ok=True)
            with tarfile.open(path,'w:gz') as stream:
                for i,(name,body) in enumerate(sources.items()):
                    if mutated and i==0:body=b'// changed despite newly authorized outer bundle hash\n'
                    entry=tarfile.TarInfo(name);entry.size=len(body);stream.addfile(entry,io.BytesIO(body))
            plan['source_bundle_bytes']=path.stat().st_size;plan['source_bundle_sha256']=replay.sha(path);retain('materials/gateway/SHA256SUMS',(plan['source_bundle_sha256']+'  ./rust-source.tar.gz\n').encode())
        bundle();self.assertEqual(len(replay.original_payload(packet,producer,plan)),100)
        retain('bin/indra-stack-acceptance',harness+b'changed')
        with self.assertRaisesRegex(ValueError,'harness SHA'):replay.original_payload(packet,producer,plan)
        retain('bin/indra-stack-acceptance',harness);retain('build/native/image.json',b'{}')
        with self.assertRaisesRegex(ValueError,'image record'):replay.original_payload(packet,producer,plan)
        retain('build/native/image.json',json.dumps(plan['images']['native']).encode());bundle(mutated=True)
        with self.assertRaisesRegex(ValueError,'compiled source differs'):replay.original_payload(packet,producer,plan)

    def test_workflow_is_read_only_fixed_branch_and_has_no_build_or_signing(self):
        workflow=yaml.safe_load((ROOT/'.github/workflows/kubernetes-release-replay.yml').read_text())
        self.assertEqual(workflow['permissions'],{'contents':'read','actions':'read','packages':'read'})
        self.assertEqual(workflow.get('on',workflow.get(True)),{'push':{'branches':['diagnostic/kubernetes-39213042394']}})
        self.assertFalse(workflow['concurrency']['cancel-in-progress']);self.assertEqual(workflow['jobs']['replay']['timeout-minutes'],60)
        job=workflow['jobs']['replay'];self.assertNotIn('env',job)
        init=next(i for i,value in enumerate(job['steps']) if value.get('name')=='Set the diagnostic evidence directory')
        admitted=next(i for i,value in enumerate(job['steps']) if value.get('name','').startswith('Admit the full original'))
        self.assertLess(init,admitted);self.assertIn('$RUNNER_TEMP',job['steps'][init]['run']);self.assertIn('$GITHUB_ENV',job['steps'][init]['run'])
        for step in workflow['jobs']['replay']['steps']:
            if 'run' in step:
                result=subprocess.run(['bash','-n'],input=step['run'],text=True,capture_output=True);self.assertEqual(result.returncode,0,result.stderr)
                for forbidden in ('docker build','docker push','crane push','cosign sign','cosign attest','--privileged','--pid=host'):
                    self.assertNotIn(forbidden,step['run'])
        self.assertEqual(set(PLAN['images']),{'backend','bridge','gateway','native','tesseract','tika'})
        self.assertEqual(PLAN['recorder_sha256'],replay.sha(ROOT/'scripts/record-cri-version.sh'))


if __name__=='__main__':unittest.main()
