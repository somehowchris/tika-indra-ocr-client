#!/usr/bin/env python3
"""Read-only admission of an explicitly pinned SDK/Chef producer for current source."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import sys
import time
import math

ROOT=Path(__file__).resolve().parent.parent
GENERATED='docker/native-inputs.generated.Dockerfile'
PROOF_JOB='Fresh hosted SDK and Chef seven-role proof'
PREDICATE='https://github.com/somehowchris/tika-indra-ocr-client/attestations/native-build-inputs/v1'


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


index_api=module('native_input_index',ROOT/'scripts/native-build-index.py')
proof_api=module('native_input_predicate',ROOT/'scripts/native-build-proof.py')
chef_api=module('native_input_chef_key',ROOT/'scripts/chef-artifact.py')


def require(value,message):
    if not value:raise ValueError(message)


def read(path):return index_api.read(path)


def write(path,value):return index_api.write(path,value)


def digest(value):return isinstance(value,str) and re.fullmatch(r'sha256:[0-9a-f]{64}',value) is not None


def checksum(value):return isinstance(value,str) and re.fullmatch(r'[0-9a-f]{64}',value) is not None


def positive(value):return type(value) is int and value>0


def validate_lock(value,repository):
    require(isinstance(value,dict),'consumer lock must be an object')
    require(set(value)=={'schema_version','kind','producer','producer_artifact','metadata_directory','index_sha256','index_bundle_sha256','inputs','mode','admission'},'unexpected consumer lock fields')
    require(type(value['schema_version']) is int and value['schema_version']==1 and value['kind']=='admitted-native-build-inputs' and value['mode'] in ('sdk','chef'),'unsupported native consumer lock')
    require(isinstance(value['metadata_directory'],str),'relative metadata directory required')
    directory=Path(value['metadata_directory'])
    require(directory.parts and not directory.is_absolute() and '..' not in directory.parts and directory.as_posix()==value['metadata_directory'],'relative metadata directory required')
    producer=value['producer']
    require(isinstance(producer,dict),'producer identity must be an object')
    require(set(producer)=={'repository','source_sha','source_ref','release_run_id','release_run_attempt','workflow_ref','workflow_identity','oidc_issuer'},'producer identity fields differ')
    require(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+',repository) is not None and producer['repository']==repository,'producer repository differs from consumer repository')
    require(isinstance(producer['source_sha'],str) and re.fullmatch('[0-9a-f]{40}',producer['source_sha']) is not None,'producer source SHA missing')
    require(isinstance(producer['source_ref'],str) and re.fullmatch(r'refs/tags/v[A-Za-z0-9_./-]+',producer['source_ref']) is not None,'existing trusted v-tag producer required')
    workflow=f'{repository}/.github/workflows/release.yml@{producer["source_ref"]}'
    require(producer['workflow_ref']==workflow and producer['workflow_identity']=='https://github.com/'+workflow and producer['oidc_issuer']=='https://token.actions.githubusercontent.com','producer signing policy differs')
    for name in ('release_run_id','release_run_attempt'):
        require(isinstance(producer[name],str) and re.fullmatch('[1-9][0-9]*',producer[name]) is not None,'producer run/attempt missing')
    require(checksum(value['index_sha256']) and checksum(value['index_bundle_sha256']),'exact signed index and bundle hashes required')
    prefix='ghcr.io/'+repository.split('/')[0].lower()+'/tika-indra-native-'
    require(set(value['inputs'])=={'sdk','chef'},'both immutable build-input references required')
    for role,reference in value['inputs'].items():
        require(isinstance(reference,str) and reference==prefix+role+'@'+reference.split('@')[-1] and digest(reference.split('@')[-1]),'unexpected immutable input reference')
    artifact=value['producer_artifact'];admission=value['admission']
    require(set(artifact)=={'id','digest'} and positive(artifact['id']) and digest(artifact['digest']),'producer metadata artifact identity missing')
    require(set(admission)=={'proof_run_attempt','proof_job_id','proof_artifact_id','proof_artifact_digest','proof_result_sha256','cost_result_sha256','cost_protocol_sha256','reviewed_net_cost'},'fresh proof and reviewed cost receipt fields required')
    require(isinstance(admission['proof_run_attempt'],str) and re.fullmatch('[1-9][0-9]*',admission['proof_run_attempt']) is not None and int(admission['proof_run_attempt'])>=int(producer['release_run_attempt']),'proof attempt precedes producer')
    require(positive(admission['proof_job_id']) and positive(admission['proof_artifact_id']) and digest(admission['proof_artifact_digest']),'fresh proof artifact/job identities missing')
    require(all(checksum(admission[name]) for name in ('proof_result_sha256','cost_result_sha256','cost_protocol_sha256')),'reviewed actual proof/cost receipt hashes missing')
    require(isinstance(admission['reviewed_net_cost'],str) and 1<=len(admission['reviewed_net_cost'])<=2048,'reviewed net-cost decision required')
    return producer


def copy_metadata(source,packet,names):
    total=0
    for name in names:
        original=source/name
        require(original==original.resolve() and original.is_file() and not original.is_symlink(),'metadata path crosses a symlink or is not a regular file')
        size=original.stat().st_size;total+=size
        require(size<=16777216 and total<=67108864,'bounded public metadata exceeds16MiB per file or64MiB total')
        read(original)
        destination=packet/name;destination.parent.mkdir(parents=True,exist_ok=True);destination.write_bytes(original.read_bytes())


def reviewed_results(packet,lock,index):
    admission=lock['admission']
    for name,key in (('proof-result.json','proof_result_sha256'),('cost-result.json','cost_result_sha256'),('cost-protocol.json','cost_protocol_sha256')):
        require(index_api.sha(packet/name)==admission[key],'reviewed admission receipt differs: '+name)
    proof=read(packet/'proof-result.json');cost=read(packet/'cost-result.json');protocol=read(packet/'cost-protocol.json')
    require(proof.get('passed') is True and proof.get('source_sha')==index['source_sha'] and proof.get('arms')==['sdk','chef'] and proof.get('roles_per_arm')==7 and proof.get('cpu_caps')==[1,2] and proof.get('source_materials_equal') is True and proof.get('chef_invalidation_passed') is True and proof.get('production_candidate_images')==6,'actual complete fresh SDK/Chef correctness proof required')
    require(protocol.get('source_sha')==index['source_sha'] and protocol.get('inputs')==index['inputs'] and protocol.get('targets')==['engine-acceptance','server-acceptance'],'reviewed cost protocol belongs to different inputs/source')
    require(cost.get('passed') is True and cost.get('timed_builds')==36 and len(cost.get('paired_differences',[]))==9,'actual complete paired cost result required')
    summaries=cost.get('summaries',{})
    require(set(summaries)=={'warm','source-comment','material-doc'},'reviewed cost scenarios differ')
    for values in summaries.values():
        require(set(values)=={'sdk','chef'},'reviewed cost arm set differs')
        for result in values.values():
            numbers=[result.get(name) for name in ('min_seconds','median_seconds','max_seconds')]
            require(result.get('n')==3 and all(type(v) in (float,int) and math.isfinite(v) and v>=0 for v in numbers) and numbers==sorted(numbers),'three finite ordered cost repetitions required')


def chef_compatibility(packet,index,source):
    key=read(packet/'chef-manifests/key.json');inputs=key['inputs'];expected=index['inputs']['chef']['recipe_sha256']
    require(key.get('dependency_key_sha256')==expected and hashlib.sha256(chef_api.canonical(inputs)).hexdigest()==expected,'signed Chef dependency key metadata differs')
    require(inputs.get('sdk_recipe_sha256')==index['inputs']['sdk']['recipe_sha256'] and inputs.get('sdk_content_sha256')==index['inputs']['sdk']['content_sha256'],'Chef/native SDK compatibility boundary differs')
    tools={name:index_api.sha(source/'scripts'/name) for name in ('chef-artifact.py','render-chef-consumer.py','cargo-chef.lock.json')}
    require(inputs.get('workspace_inputs')==chef_api.source_inputs(source) and inputs.get('build_tools')==tools,'Chef source/lock/features/profile/configuration key changed; explicitly select SDK-only compilation before restoring Chef. No SDK republish is required for Rust-only changes.')


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--lock',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--consumer-mode',choices=('locked','sdk'),default='locked');args=parser.parse_args()
    require(args.lock.is_file() and not args.lock.is_symlink() and args.lock.stat().st_size<=65536,'regular bounded supplied lock required; no fallback after admission failure')
    require(not args.output.exists() and not args.output.is_symlink(),'exclusive admission output required')
    output=args.output.resolve();require('\n' not in str(output) and '\r' not in str(output),'invalid output path');output.mkdir(mode=0o700)
    def run(name,command,seconds=180,maximum=16777216,binary=False):
        started=time.monotonic();exit_code=None
        def bounded_files():resource.setrlimit(resource.RLIMIT_FSIZE,(maximum,maximum))
        with (output/(name+'.stdout')).open('xb') as stdout,(output/(name+'.stderr')).open('xb') as stderr:
            try:exit_code=subprocess.run([str(v) for v in command],cwd=ROOT,stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr,timeout=seconds,preexec_fn=bounded_files).returncode
            except subprocess.TimeoutExpired:exit_code=124
        write(output/(name+'.json'),{'command':[str(v) for v in command],'exit_code':exit_code,'elapsed_seconds':time.monotonic()-started,'scope':'admission/transport diagnostic, not an isolated speedup or daemon CPU measurement'})
        require(exit_code==0,'native input admission command failed: '+name)
        return None if binary else (output/(name+'.stdout')).read_text()
    try:
        consumer=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
        require(consumer==os.environ['GITHUB_SHA'],'consumer checkout SHA differs')
        lock=read(args.lock);producer=validate_lock(lock,os.environ['GITHUB_REPOSITORY']);mode='sdk' if args.consumer_mode=='sdk' else lock['mode']
        (output/'lock.json').write_bytes(args.lock.read_bytes())
        admission=lock['admission'];repository=producer['repository'];producer_run=producer['release_run_id'];proof_attempt=admission['proof_run_attempt']
        # Successful proof/cost data is reviewed when pinning. Every consumer
        # checks the exact completed GitHub job again. Bounded signed metadata
        # and reviewed summaries remain beside the lock; no 90-day artifact
        # expiry or large proof download is introduced into routine builds.
        api='repos/'+repository+'/actions/'
        body=run('proof-attempt',['gh','api',api+'runs/'+producer_run+'/attempts/'+proof_attempt]);attempt=json.loads(body)
        require(attempt.get('head_sha')==producer['source_sha'] and attempt.get('id')==int(producer_run) and attempt.get('run_attempt')==int(proof_attempt) and attempt.get('path')=='.github/workflows/release.yml' and attempt.get('status')=='completed' and attempt.get('conclusion')=='success' and attempt.get('event') in ('push','workflow_dispatch'),'successful trusted producer proof attempt required')
        body=run('proof-job',['gh','api',api+'jobs/'+str(admission['proof_job_id'])]);job=json.loads(body)
        require(job.get('run_id')==int(producer_run) and job.get('run_attempt')==int(proof_attempt) and job.get('head_sha')==producer['source_sha'] and job.get('name')==PROOF_JOB and job.get('status')=='completed' and job.get('conclusion')=='success','successful exact fresh-hosted proof job required')
        source=args.lock.resolve().parent/lock['metadata_directory']
        require(source.is_dir() and not source.is_symlink() and source==source.resolve(),'canonical local signed metadata directory required')
        packet=output/'metadata';packet.mkdir()
        copy_metadata(source,packet,('index.json','index.sigstore.json','sdk.spdx.json','chef.spdx.json','sdk-materialized/materialization.json','chef-oci-integrity.json','chef-manifests/key.json','proof-result.json','cost-result.json','cost-protocol.json'))
        require(index_api.sha(packet/'index.json')==lock['index_sha256'] and index_api.sha(packet/'index.sigstore.json')==lock['index_bundle_sha256'],'pinned signed index/bundle differs')
        verify=['--certificate-identity',producer['workflow_identity'],'--certificate-oidc-issuer',producer['oidc_issuer'],
                '--certificate-github-workflow-repository',repository,'--certificate-github-workflow-ref',producer['source_ref'],'--certificate-github-workflow-sha',producer['source_sha']]
        run('index-signature',['cosign','verify-blob','--bundle',packet/'index.sigstore.json',*verify,packet/'index.json'])
        index=read(packet/'index.json');index_api.validate(index,producer)
        require({role:entry['reference'] for role,entry in index['inputs'].items()}==lock['inputs'],'locked immutable inputs differ from signed index')
        reviewed_results(packet,lock,index)
        if mode=='chef':chef_compatibility(packet,index,ROOT)
        for role in (('sdk','chef') if mode=='chef' else ('sdk',)):
            entry=index['inputs'][role];reference=entry['reference'];sbom=packet/(role+'.spdx.json')
            require(index_api.sha(sbom)==entry['spdx_sha256'],'signed SBOM bytes differ')
            receipt=packet/('sdk-materialized/materialization.json' if role=='sdk' else 'chef-oci-integrity.json')
            require(index_api.sha(receipt)==entry['oci_receipt_sha256'],'signed producer raw OCI receipt differs')
            run(role+'-signature',['cosign','verify',*verify,reference])
            for kind,name,expected in (('spdxjson','sbom',read(sbom)),(PREDICATE,'inputs',index)):
                body=run(role+'-'+name,['cosign','verify-attestation',*verify,'--type',kind,reference]);proof_api.require_attestation(body,expected,entry['digest'])
            run(role+'-download',['crane','pull','--format=oci',reference,output/(role+'-oci')],1800,8589934592)
        sdk=index['inputs']['sdk']
        run('sdk-materialize',['python3',ROOT/'scripts/native-sdk.py','materialize-oci','--layout',output/'sdk-oci','--digest',sdk['digest'],'--inputs',ROOT,'--output',output/'sdk-materialized'],1800,8589934592)
        actual=read(output/'sdk-materialized/materialization.json')
        require(actual['recipe_sha256']==sdk['recipe_sha256'] and actual['content_sha256']==sdk['content_sha256'],'SDK recipe/content differs')
        if mode=='chef':run('chef-distribution',['python3',ROOT/'scripts/native-sdk.py','verify-oci','--layout',output/'chef-oci','--digest',index['inputs']['chef']['digest'],'--output',output/'chef-distribution.json'],1800)
        recipe=ROOT/GENERATED;require(not recipe.exists() and not recipe.is_symlink(),'generated consumer recipe already exists')
        if mode=='chef':run('render-consumer',['python3',ROOT/'scripts/render-chef-consumer.py','--source',ROOT/'docker/native.Dockerfile','--mode','consumer','--output',recipe],60)
        else:run('render-consumer',['python3',ROOT/'scripts/native-sdk.py','render-consumer','--source',ROOT/'docker/native.Dockerfile','--output',recipe],60)
        (output/'consumer.Dockerfile').write_bytes(recipe.read_bytes())
        contexts=['sdk-input=oci-layout://'+str(output/'sdk-oci')+'@'+sdk['digest']]
        arguments=['BUILD_CPUSET=0,1','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256']]
        if mode=='chef':
            chef=index['inputs']['chef'];contexts+=['chef-input=oci-layout://'+str(output/'chef-oci')+'@'+chef['digest']]
            arguments+=['CHEF_DEPENDENCY_KEY_SHA256='+chef['recipe_sha256'],'CHEF_CONTENT_SHA256='+chef['content_sha256']]
        write(output/'result.json',{'schema_version':1,'passed':True,'mode':mode,'producer':producer,'consumer_source_sha':consumer,
            'index_sha256':index_api.sha(packet/'index.json'),'dockerfile':GENERATED,'rendered_recipe_sha256':index_api.sha(recipe),'build_contexts':contexts,'build_args':arguments,
            'reviewed_admission':admission,'scope':'verified immutable build input; current SDK recipe/toolchain/cache content are additionally admitted inside native-build/chef-restored before compilation',
            'performance_claim':False,'current_application_equivalence_claim':False,'implicit_source_fallback':False,'gha_cache_export_enabled':False})
    finally:
        error=sys.exc_info()[1]
        write(output/'completion.json',{'schema_version':1,'passed':error is None,'failure':str(error) if error else None,'source_fallback_attempted':False})


if __name__=='__main__':main()
