#!/usr/bin/env python3
"""Admit one frozen failed-release artifact for a read-only Kubernetes diagnostic."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import shutil
import stat
import subprocess
import tarfile
import zipfile

ROOT=Path(__file__).resolve().parent.parent
PLAN=ROOT/'evaluation/kubernetes/replay-39213042394.json'
REF='refs/heads/diagnostic/kubernetes-39213042394'
HELPERS=('scripts/verify-kubernetes-native.sh','scripts/verify-daemon-image.sh','scripts/verify-cri-image.sh','scripts/verify-image-provenance.sh')


def require(value,message):
    if not value:raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):
    require(path==path.resolve() and path.is_file() and not path.is_symlink() and path.stat().st_size<=16777216,'canonical bounded JSON required')
    return json.loads(path.read_text())


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False);stream.write('\n')


def git(source,*arguments):return subprocess.check_output(['git',*arguments],cwd=source)


def context(plan):
    require(os.environ.get('GITHUB_REPOSITORY')==plan['repository'] and os.environ.get('GITHUB_EVENT_NAME')=='push' and os.environ.get('GITHUB_REF')==REF and os.environ.get('GITHUB_JOB')=='replay','only the fixed canonical diagnostic branch-push job is permitted')
    require(os.environ.get('GITHUB_WORKFLOW_REF')==plan['repository']+'/.github/workflows/kubernetes-release-replay.yml@'+REF,'unexpected diagnostic workflow')
    require(git(ROOT,'rev-parse','HEAD').decode().strip()==os.environ['GITHUB_SHA'],'current controller checkout differs')
    require(subprocess.run(['git','diff','--quiet','HEAD','--'],cwd=ROOT).returncode==0,'current controller checkout is dirty')


def validate_api(run,job,artifact,plan):
    require(run.get('id')==plan['run_id'] and run.get('run_attempt')==plan['run_attempt'] and run.get('head_sha')==plan['producer_source_sha'] and run.get('head_branch')==plan['producer_ref'].removeprefix('refs/tags/') and run.get('path')=='.github/workflows/release.yml' and run.get('event')=='workflow_dispatch' and run.get('status')=='completed' and run.get('conclusion')=='failure','original failed release identity differs')
    require(run.get('repository',{}).get('full_name')==plan['repository'] and run.get('head_repository',{}).get('full_name')==plan['repository'],'original release repository differs')
    require(job.get('id')==plan['publish_job_id'] and job.get('run_id')==plan['run_id'] and job.get('run_attempt')==plan['run_attempt'] and job.get('head_sha')==plan['producer_source_sha'] and job.get('name')=='publish' and job.get('status')=='completed' and job.get('conclusion')=='failure','original failed publish job differs')
    failed=[value for value in job.get('steps',[]) if value.get('conclusion')=='failure']
    require(len(failed)==1 and failed[0].get('name')=='Finish accepted native OCR during real Kubernetes sidecar shutdown','original failure was not the frozen Kubernetes gate')
    require(artifact.get('id')==plan['artifact_id'] and artifact.get('name')==plan['artifact_name'] and artifact.get('size_in_bytes')==plan['artifact_bytes'] and artifact.get('digest')=='sha256:'+plan['artifact_sha256'] and artifact.get('expired') is False and artifact.get('workflow_run',{}).get('id')==plan['run_id'] and artifact.get('workflow_run',{}).get('head_sha')==plan['producer_source_sha'],'original exact artifact identity differs')


def unpack(archive,output,expected_bytes,expected_sha,names):
    require(archive==archive.resolve() and archive.is_file() and not archive.is_symlink() and archive.stat().st_size==expected_bytes and sha(archive)==expected_sha,'full original artifact length/SHA-256 differs')
    require(not output.exists(),'exclusive diagnostic extraction required');output.mkdir(mode=0o700)
    with zipfile.ZipFile(archive) as source:
        entries=source.infolist();seen=set();total=0
        require(len(entries)<=10000,'ZIP member bound exceeded')
        for entry in entries:
            path=Path(entry.filename);mode=entry.external_attr>>16;total+=entry.file_size
            require(path.parts and not path.is_absolute() and '..' not in path.parts and '\\' not in entry.filename and entry.filename not in seen and not stat.S_ISLNK(mode),'unsafe/duplicate ZIP member')
            require(not mode&0o7000 and (stat.S_IFMT(mode) in (0,stat.S_IFREG,stat.S_IFDIR)) and entry.file_size<=268435456 and total<=1073741824,'ZIP type/size bound exceeded');seen.add(entry.filename)
        require(set(names)<=seen,'required original diagnostic evidence is missing')
        for name in names:
            destination=output/name;destination.parent.mkdir(parents=True,exist_ok=True)
            with source.open(name) as original,destination.open('xb') as target:shutil.copyfileobj(original,target,1048576)


def controller_closure(producer,controller,recorder_sha,gate_delta):
    names=lambda path:set(git(path,'ls-files','-z','--','deploy').decode().rstrip('\0').split('\0'))
    original=names(producer);current=names(controller)
    require(original==current and '' not in original,'current deployment file inventory differs')
    require(set(gate_delta)=={'scripts/verify-kubernetes-native.sh','scripts/lib/kind-cgroup.sh'},'unexpected reviewed gate exception inventory')
    values=[]
    for name in sorted(original|set(HELPERS)):
        left=producer/name;right=controller/name
        require(left==left.resolve() and right==right.resolve() and left.is_file() and right.is_file(),'noncanonical controller/OCI/deployment closure: '+name)
        if name=='scripts/verify-kubernetes-native.sh':
            delta=gate_delta[name]
            require(set(delta)=={'producer_sha256','controller_sha256'} and sha(left)==delta['producer_sha256'] and sha(right)==delta['controller_sha256'],'exact reviewed old/new Kubernetes gate differs')
        else:require(left.read_bytes()==right.read_bytes(),'current controller/OCI/deployment closure differs: '+name)
        values.append({'path':name,'sha256':sha(right)})
    name='scripts/lib/kind-cgroup.sh';left=producer/name;right=controller/name;delta=gate_delta[name]
    require(set(delta)=={'producer_absent','controller_sha256'} and delta['producer_absent'] is True and not left.exists() and not left.is_symlink() and right==right.resolve() and right.is_file() and sha(right)==delta['controller_sha256'],'exact reviewed new cgroup helper differs')
    values.append({'path':name,'sha256':sha(right)})
    require(sha(controller/'scripts/record-cri-version.sh')==recorder_sha,'current CRI recorder differs from the exact reviewed fix')
    return values


def original_payload(packet,producer,plan):
    require(git(producer,'rev-parse','HEAD').decode().strip()==plan['producer_source_sha'] and subprocess.run(['git','diff','--quiet','HEAD','--'],cwd=producer).returncode==0,'original producer checkout differs')
    harness=packet/'bin/indra-stack-acceptance';require(sha(harness)==plan['harness_sha256'],'original harness SHA-256 differs')
    require((packet/'bin/indra-stack-acceptance.sha256').read_text().split()[0]==plan['harness_sha256'],'original harness hash receipt differs')
    lines=(packet/'acceptance-build/gateway/binaries.sha256').read_text().splitlines()
    require([line for line in lines if line.endswith('  /out/indra-stack-acceptance')]==[plan['harness_sha256']+'  /out/indra-stack-acceptance'],'actual original compilation manifest differs')
    for role,expected in plan['images'].items():require(read(packet/'build'/role/'image.json')==expected,'original immutable image record differs: '+role)
    require(read(packet/'prepare-result.json').get('passed') is True,'original image/material preparation did not pass')
    bundle=packet/'materials/gateway/rust-source.tar.gz'
    require(bundle.stat().st_size==plan['source_bundle_bytes'] and sha(bundle)==plan['source_bundle_sha256'],'original complete source bundle differs')
    require([line for line in (packet/'materials/gateway/SHA256SUMS').read_text().splitlines() if line.endswith('  ./rust-source.tar.gz')]==[plan['source_bundle_sha256']+'  ./rust-source.tar.gz'],'original source bundle receipt differs')
    names=git(producer,'ls-files','-z').decode().rstrip('\0').split('\0')
    selected={name for name in names if name.endswith(('.rs','Cargo.toml','Cargo.lock')) and (name.startswith(('src/','tests/','benches/','runtime/','shared/')) or name in ('Cargo.toml','Cargo.lock')) or name.startswith('backend/tests/data/') or name=='backend/models.lock.json' or name.startswith('runtime/.cargo/')}
    selected={name for name in selected if not name.startswith('runtime/proofs/')}
    recorded={line.split('  ',1)[1]:line.split('  ',1)[0] for line in (packet/'source-files.sha256').read_text().splitlines()}
    found={};total=0;seen=set()
    with tarfile.open(bundle,'r:gz') as source:
        for entry in source:
            path=Path(entry.name);total+=entry.size
            require(path.parts and not path.is_absolute() and '..' not in path.parts and entry.name not in seen and (entry.isfile() or entry.isdir()) and len(seen)<50000 and total<=1073741824,'unsafe/oversized original source bundle');seen.add(entry.name)
            if entry.name in selected:
                require(entry.isfile() and entry.size<=16777216,'invalid original compiled source entry')
                body=source.extractfile(entry).read();expected=(producer/entry.name).read_bytes();digest=hashlib.sha256(body).hexdigest()
                require(body==expected and recorded.get(entry.name)==digest,'original compiled source differs: '+entry.name);found[entry.name]=digest
    require(set(found)==selected and len(found)>=100,'original Rust/manifests/fixture source closure incomplete')
    return [{'path':name,'sha256':found[name]} for name in sorted(found)]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--producer',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    plan=read(PLAN);context(plan);require(not args.output.exists(),'exclusive replay output required');args.output.mkdir(mode=0o700);output=args.output.resolve()
    def fetch(name,path,limit=16777216,timeout=180):
        def bound():resource.setrlimit(resource.RLIMIT_FSIZE,(limit,limit))
        with (output/name).open('xb') as stdout,(output/(name+'.stderr')).open('xb') as stderr:status=subprocess.run(['gh','api',path],stdout=stdout,stderr=stderr,stdin=subprocess.DEVNULL,timeout=timeout,preexec_fn=bound).returncode
        require(status==0,'read-only original evidence download failed: '+name)
    try:
        prefix='repos/'+plan['repository']+'/actions/'
        fetch('run.json',prefix+'runs/'+str(plan['run_id'])+'/attempts/'+str(plan['run_attempt']))
        fetch('job.json',prefix+'jobs/'+str(plan['publish_job_id']));fetch('artifact.json',prefix+'artifacts/'+str(plan['artifact_id']))
        validate_api(read(output/'run.json'),read(output/'job.json'),read(output/'artifact.json'),plan)
        closure=controller_closure(args.producer.resolve(),ROOT,plan['recorder_sha256'],plan['reviewed_gate_delta'])
        fetch('producer.zip',prefix+'artifacts/'+str(plan['artifact_id'])+'/zip',1073741824,1800)
        names=['bin/indra-stack-acceptance','bin/indra-stack-acceptance.sha256','acceptance-build/gateway/binaries.sha256','materials/gateway/rust-source.tar.gz','materials/gateway/SHA256SUMS','source-files.sha256','prepare-result.json',*('build/'+role+'/image.json' for role in plan['images'])]
        unpack(output/'producer.zip',output/'original',plan['artifact_bytes'],plan['artifact_sha256'],names)
        compiled=original_payload(output/'original',args.producer.resolve(),plan)
        shutil.copyfile(PLAN,output/'plan.json')
        write(output/'admission.json',{'schema_version':1,'passed':True,'controller_source_sha':os.environ['GITHUB_SHA'],'producer_source_sha':plan['producer_source_sha'],'harness_sha256':plan['harness_sha256'],'complete_artifact_sha256':plan['artifact_sha256'],'complete_source_bundle_sha256':plan['source_bundle_sha256'],'compiled_source_closure':compiled,'controller_gate_closure':closure,'reviewed_recorder_sha256':plan['recorder_sha256'],'reviewed_gate_delta':plan['reviewed_gate_delta'],'gate_fix_source_commit':plan['gate_fix_source_commit'],'gate_fix_followup_commit':plan['gate_fix_followup_commit'],'images':plan['images'],'pulled_roles':['native','tika'],'signing_claim':False,'release_admission_claim':False,'performance_claim':False,'scope':plan['scope']})
    finally:
        import sys
        error=sys.exc_info()[1];write(output/'prepare-completion.json',{'passed':error is None,'failure':str(error) if error else None})


if __name__=='__main__':main()
