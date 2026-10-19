#!/usr/bin/env python3
"""Admit the one frozen source41 failure for a read-only trace-gate diagnostic."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import stat
import subprocess
import sys
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = 'somehowchris/tika-indra-ocr-client'
PRODUCER = '60c849a0f829c7813c5e3fbfb2dc6e907eda1958'
PRODUCER_TAG = 'v0.0.0-validation-20261019-60c849a'
REF = 'refs/heads/diagnostic/trace-source41-20261019'
RUN = 39232133426
JOB = 117992987385
ARTIFACT = 12184306509
ARTIFACT_BYTES = 684001410
ARTIFACT_SHA = 'a31b5257f3d61fc03d24af34d370c544b737c54dcab34831e557c6a6f94ca1c2'
FAILED_STEP = 'Verify actual gateway, forked Java, bridge and native OTLP graph before signing'
UNCHANGED = ('scripts/verify-trace-native.sh','scripts/verify-release-traces.sh','scripts/verify-daemon-image.sh','scripts/verify-image-provenance.sh','docker/gateway.Dockerfile','backend/models.lock.json')
# Exact packaging delta reviewed for this fixed diagnostic; other trace/OCI/build helpers remain byte-identical.
REVIEWED = {'docker/trace-acceptance.Dockerfile': {'producer_sha256': '4a987438be098df6209e73c43e2de56f5f4be54792dffd6d1f4e466671912aee', 'controller_sha256': '0dd9040c5830b090118cc00723cd2faecdaf1a9d105ec1198dea9ab4c490e42e'},
            'scripts/prepare-trace-helper.sh': {'producer_absent': True, 'controller_sha256': '81374f755e1f342788121319901781fb67fd2fbdd2d75017ae2bd616f349ec38'}}
ROLES = {'native':('docker/native.Dockerfile','server'),'gateway':('docker/gateway.Dockerfile','gateway'),
         'bridge':('docker/bridge.Dockerfile',''),'tika':('docker/tika.Dockerfile','runtime'),
         'tesseract':('docker/tesseract.Dockerfile','runtime'),'backend':('backend/Dockerfile','')}


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):
    path = Path(path)
    require(path == path.resolve() and path.is_file() and not path.is_symlink() and path.stat().st_size <= 16777216, 'canonical bounded JSON required: '+str(path))
    return json.loads(path.read_text())


def write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False)
        stream.write('\n')


def git(source,*arguments):
    return subprocess.check_output(['git',*arguments],cwd=source)


def context():
    require(os.environ.get('GITHUB_REPOSITORY') == REPOSITORY and os.environ.get('GITHUB_EVENT_NAME') == 'push' and os.environ.get('GITHUB_REF') == REF and os.environ.get('GITHUB_JOB') == 'replay', 'only fixed canonical diagnostic branch-push job permitted')
    require(os.environ.get('GITHUB_WORKFLOW_REF') == REPOSITORY+'/.github/workflows/trace-diagnostic.yml@'+REF, 'unexpected diagnostic workflow')
    require(git(ROOT,'rev-parse','HEAD').decode().strip() == os.environ['GITHUB_SHA'] and not git(ROOT,'status','--porcelain'), 'current controller checkout differs or is dirty')
    require(all(re.fullmatch('[0-9a-f]{64}', record['controller_sha256']) for record in REVIEWED.values()), 'reviewed packaging hashes are not frozen')


def validate_api(run,job,artifact):
    require(run.get('id') == RUN and run.get('run_attempt') == 1 and run.get('head_sha') == PRODUCER and run.get('head_branch') == PRODUCER_TAG and run.get('path') == '.github/workflows/release.yml' and run.get('event') == 'workflow_dispatch' and run.get('status') == 'completed' and run.get('conclusion') == 'failure', 'original failed release identity differs')
    require(run.get('repository',{}).get('full_name') == REPOSITORY and run.get('head_repository',{}).get('full_name') == REPOSITORY, 'original release repository differs')
    require(job.get('id') == JOB and job.get('run_id') == RUN and job.get('run_attempt') == 1 and job.get('head_sha') == PRODUCER and job.get('name') == 'publish' and job.get('status') == 'completed' and job.get('conclusion') == 'failure', 'original failed publish job differs')
    failed = [step for step in job.get('steps',[]) if step.get('conclusion') == 'failure']
    require(len(failed) == 1 and failed[0].get('name') == FAILED_STEP, 'original failure is not the exact trace image-assembly step')
    required = ('Bind exact candidate images to static checks and complete source materials','Run real native and gateway process gates at 1 and 2 CPU','Finish accepted native OCR during real Compose shutdown','Finish accepted native OCR during real Kubernetes sidecar shutdown','Verify the exact Python-reference and Tesseract Tika candidates at both caps')
    for name in required:
        matches = [step for step in job.get('steps',[]) if step.get('name') == name]
        require(len(matches) == 1 and matches[0].get('conclusion') == 'success', 'original prerequisite did not pass: '+name)
    require(artifact.get('id') == ARTIFACT and artifact.get('name') == f'signed-release-evidence-{PRODUCER}-{RUN}-1' and artifact.get('size_in_bytes') == ARTIFACT_BYTES and artifact.get('digest') == 'sha256:'+ARTIFACT_SHA and artifact.get('expired') is False and artifact.get('workflow_run',{}).get('id') == RUN and artifact.get('workflow_run',{}).get('head_sha') == PRODUCER, 'original artifact identity differs')


def closure(producer):
    require(git(producer,'rev-parse','HEAD').decode().strip() == PRODUCER and not git(producer,'status','--porcelain'), 'original source41 checkout differs or is dirty')
    result = []
    for name in UNCHANGED:
        left,right = producer/name,ROOT/name
        require(left == left.resolve() and right == right.resolve() and left.is_file() and right.is_file() and left.read_bytes() == right.read_bytes(), 'trace/OCI/helper-build closure differs: '+name)
        result.append({'path':name,'producer_sha256':sha(left),'controller_sha256':sha(right),'unchanged':True})
    for name,expected in REVIEWED.items():
        left,right = producer/name,ROOT/name
        require(right == right.resolve() and right.is_file() and sha(right) == expected['controller_sha256'], 'reviewed new packaging bytes differ: '+name)
        if expected.get('producer_absent') is True:
            require(not left.exists() and not left.is_symlink(), 'new helper unexpectedly exists in old producer')
        else:
            require(left.is_file() and sha(left) == expected['producer_sha256'], 'reviewed original packaging bytes differ')
        result.append({'path':name,**expected})
    return result


def unpack(archive,output):
    require(archive == archive.resolve() and archive.is_file() and not archive.is_symlink() and archive.stat().st_size == ARTIFACT_BYTES and sha(archive) == ARTIFACT_SHA, 'full original ZIP bytes/SHA256 differ')
    require(not output.exists(), 'exclusive extraction required')
    output.mkdir(mode=0o700)
    names = {'source-files.sha256','prepare-result.json','process/result.json','references/result.json','acceptance-build/gateway/binaries.sha256','materials/gateway/rust-source.tar.gz','materials/gateway/SHA256SUMS','materials/tika/java/tika-server.jar','bin/indra-server','bin/indra-gateway','bin/indra-tesseract',*('build/'+role+'/image.json' for role in ROLES)}
    with zipfile.ZipFile(archive) as source:
        entries = source.infolist(); seen = set(); total = 0
        require(len(entries) <= 20000, 'ZIP member bound exceeded')
        for entry in entries:
            path = Path(entry.filename); mode = entry.external_attr >> 16; total += entry.file_size
            require(path.parts and not path.is_absolute() and '..' not in path.parts and '\\' not in entry.filename and entry.filename not in seen and not stat.S_ISLNK(mode), 'unsafe/duplicate ZIP path')
            require(not mode & 0o7000 and stat.S_IFMT(mode) in (0,stat.S_IFREG,stat.S_IFDIR) and entry.file_size <= 536870912 and total <= 2147483648, 'ZIP type/size bound exceeded')
            seen.add(entry.filename)
        require(names <= seen, 'required original trace diagnostic evidence missing')
        for name in sorted(names):
            destination = output/name; destination.parent.mkdir(parents=True,exist_ok=True)
            with source.open(name) as original, destination.open('xb') as target:
                shutil.copyfileobj(original,target,1048576)


def original_payload(packet,producer):
    for name in ('prepare-result.json','process/result.json','references/result.json'):
        require(read(packet/name).get('passed') is True, 'original gate prerequisite failed: '+name)
    images = {}
    for role,(dockerfile,target) in ROLES.items():
        value = read(packet/'build'/role/'image.json')
        require(value.get('role') == role and value.get('dockerfile') == dockerfile and value.get('target') == target and re.fullmatch('sha256:[0-9a-f]{64}',value.get('digest','')) is not None and value.get('reference') == 'ghcr.io/somehowchris/tika-indra-'+role+'@'+value.get('digest',''), 'original image coordinates differ: '+role)
        images[role] = value
    bundle = packet/'materials/gateway/rust-source.tar.gz'
    bundle_sha = sha(bundle)
    require([line for line in (packet/'materials/gateway/SHA256SUMS').read_text().splitlines() if line.endswith('  ./rust-source.tar.gz')] == [bundle_sha+'  ./rust-source.tar.gz'], 'original gateway source bundle manifest differs')
    tracked = git(producer,'ls-files','-z').decode().rstrip('\0').split('\0')
    selected = {name for name in tracked if name.endswith(('.rs','Cargo.toml','Cargo.lock')) and (name.startswith(('src/','tests/','benches/','runtime/','shared/')) or name in ('Cargo.toml','Cargo.lock')) or name.startswith('backend/tests/data/') or name == 'backend/models.lock.json' or name.startswith('runtime/.cargo/')}
    selected = {name for name in selected if not name.startswith('runtime/proofs/')}
    recorded = {}
    for line in (packet/'source-files.sha256').read_text().splitlines():
        digest,name = line.split('  ',1)
        require(name not in recorded and re.fullmatch('[0-9a-f]{64}',digest) is not None, 'invalid original source manifest')
        recorded[name] = digest
    found = {}; seen = set(); total = 0
    with tarfile.open(bundle,'r:gz') as source:
        for entry in source:
            path = Path(entry.name); total += entry.size
            require(path.parts and not path.is_absolute() and '..' not in path.parts and entry.name not in seen and (entry.isfile() or entry.isdir()) and len(seen) < 50000 and total <= 2147483648, 'unsafe/oversized original source bundle')
            seen.add(entry.name)
            if entry.name in selected:
                require(entry.isfile() and entry.size <= 16777216, 'invalid original compiled source entry')
                body = source.extractfile(entry).read(); expected = (producer/entry.name).read_bytes(); digest = hashlib.sha256(body).hexdigest()
                require(body == expected and recorded.get(entry.name) == digest, 'original compiled source differs: '+entry.name)
                found[entry.name] = digest
    require(set(found) == selected and len(found) >= 100, 'original selected Rust/source and fixture identity incomplete')
    return images,bundle_sha,[{'path':name,'sha256':found[name]} for name in sorted(found)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--producer',type=Path,required=True); parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args(); context()
    require(not args.output.exists() and not args.output.is_symlink(), 'exclusive diagnostic output required')
    args.output.mkdir(mode=0o700); output = args.output.resolve()
    def fetch(name,path,limit=16777216,timeout=180):
        def bound():
            resource.setrlimit(resource.RLIMIT_FSIZE,(limit,limit))
        with (output/name).open('xb') as stdout, (output/(name+'.stderr')).open('xb') as stderr:
            status = subprocess.run(['gh','api',path],stdout=stdout,stderr=stderr,stdin=subprocess.DEVNULL,timeout=timeout,preexec_fn=bound).returncode
        require(status == 0, 'read-only frozen evidence download failed: '+name)
    try:
        scoped = closure(args.producer.resolve())
        prefix = 'repos/'+REPOSITORY+'/actions/'
        fetch('run.json',prefix+'runs/'+str(RUN)+'/attempts/1'); fetch('job.json',prefix+'jobs/'+str(JOB)); fetch('artifact.json',prefix+'artifacts/'+str(ARTIFACT))
        validate_api(read(output/'run.json'),read(output/'job.json'),read(output/'artifact.json'))
        fetch('producer.zip',prefix+'artifacts/'+str(ARTIFACT)+'/zip',1073741824,1800)
        unpack(output/'producer.zip',output/'original')
        images,bundle_sha,compiled = original_payload(output/'original',args.producer.resolve())
        write(output/'admission.json',{'schema_version':1,'passed':True,'controller_source_sha':os.environ['GITHUB_SHA'],'runtime_producer_source_sha':PRODUCER,'helper_compiled_source_sha':PRODUCER,'helper_build_controller_sha':os.environ['GITHUB_SHA'],'complete_artifact_sha256':ARTIFACT_SHA,'complete_artifact_bytes':ARTIFACT_BYTES,'source_bundle_sha256':bundle_sha,'selected_rust_source_and_fixture_identity':compiled,'helper_build_checkout':{'source_sha':PRODUCER,'clean':True,'scope':'The whole clean producer checkout and unchanged gateway Dockerfile bind the actual helper build; selected corresponding-source members are a narrower check, not every possible include input.'},'controller_packaging_closure':scoped,'images':images,'pulled_roles':['native','tika'],'models':'explicit fresh native41 cold bridge fetch then network-none warm reuse; original model subtree deliberately pruned','signing_claim':False,'sdk_admission_claim':False,'release_admission_claim':False,'performance_claim':False,'scope':'Focused real trace diagnostic with original source41 runtime images and a newly rebuilt source41 helper; only named-local helper image assembly changes.'})
    finally:
        error = sys.exc_info()[1]
        write(output/'prepare-completion.json',{'passed':error is None,'failure':str(error) if error else None})


if __name__ == '__main__':
    main()
