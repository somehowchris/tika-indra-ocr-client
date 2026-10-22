#!/usr/bin/env python3
"""Fresh hosted native SDK/Chef correctness proof; never a performance benchmark."""
from __future__ import annotations
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time

ROOT=Path(__file__).resolve().parent.parent


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


index_api=module('native_build_index',ROOT/'scripts/native-build-index.py')
audit=module('build_cost_audit',ROOT/'evaluation/build-cost/audit.py')
materials_api=module('native_build_materials',ROOT/'scripts/prove-frozen-wp1.py')


def require(value,message):
    if not value:raise ValueError(message)


def read(path):return json.loads(path.read_text())


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False);stream.write('\n')


def verify_materials(materials,context,sdk,expected_recipe_sha256,receipt):
    """Bind complete released originals to the actual source context and SDK."""
    require(not receipt.exists() and not receipt.is_symlink(),'exclusive material receipt required')
    try:
        materials_api.verify_manifest(materials)
        # The original archive remains retained for either outcome. Extraction is
        # temporary so repeated cost cells do not retain eighteen vendor trees.
        with tempfile.TemporaryDirectory(prefix='material-source-',dir=receipt.parent) as temporary:
            source=Path(temporary)/'source'
            materials_api.extract(materials/'rust-source.tar.gz',source)
            completeness=materials_api.verify_source_completeness(context,source)
        notices=materials_api.verify_native_notices(materials,sdk,context/'scripts/collect-runtime-materials.sh',expected_recipe_sha256)
        copies={}
        for destination,original in (('README.md','docs/runtime-relinking.md'),('relinking.md','docs/relinking.md'),
                ('LICENSE','LICENSE'),('THIRD_PARTY_NOTICES.md','THIRD_PARTY_NOTICES.md'),
                ('licenses/INDRA-MODEL-LICENSE.txt','licenses/INDRA-MODEL-LICENSE.txt'),('licenses/INDRA-CODE-LICENSE.txt','licenses/INDRA-CODE-LICENSE.txt')):
            expected=context/original;actual=materials/destination
            require(actual.is_file() and not actual.is_symlink() and actual.read_bytes()==expected.read_bytes(),'copied source material missing or stale: '+destination)
            copies[destination]={'source_path':original,'sha256':index_api.sha(actual),'bytes':actual.stat().st_size}
        value={'schema_version':1,'passed':True,'copied_source_files':copies,'source_archive_sha256':index_api.sha(materials/'rust-source.tar.gz'),
               'materials_inventory':audit.inventory(materials),'source_completeness':completeness,'native_notices':notices}
    except Exception as error:
        write(receipt,{'schema_version':1,'passed':False,'error':type(error).__name__+': '+str(error),
             'scope':'actual released materials failed completeness/currency admission; original inputs retained'})
        raise
    write(receipt,value)
    return value


def envelopes(body):
    decoder=json.JSONDecoder();result=[]
    while body.strip():
        body=body.lstrip();value,end=decoder.raw_decode(body);body=body[end:]
        result.extend(value if isinstance(value,list) else [value])
    return result


def require_attestation(body,expected,digest):
    for envelope in envelopes(body):
        if envelope.get('payloadType')!='application/vnd.in-toto+json':continue
        payload=json.loads(base64.b64decode(envelope['payload'],validate=True))
        if payload.get('predicate')==expected and any(s.get('digest',{}).get('sha256')==digest.split(':')[1] for s in payload.get('subject',[])):return
    raise ValueError('verified attestation lacks exact submitted predicate/digest')


def initial_cache_empty(body):
    # Buildx JSON may be an array or a stream of records. Empty output is zero
    # records; any actual record fails the no-imported-build-cache precondition.
    values=envelopes(body)
    require(not values,'fresh hosted builder already has build cache records')



def owned_container(record,name,token,image):
    require(record.get('Name')=='/'+name and record.get('Config',{}).get('Labels',{}).get('org.tika-indra.native-build-proof')==token and record.get('Image')==image,'refused cleanup: exact name/owner/image differs')
    identifier=record.get('Id','')
    require(len(identifier)==64 and all(c in '0123456789abcdef' for c in identifier),'refused cleanup: immutable container ID differs')
    return identifier


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--packet',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();packet=args.packet.resolve();output=args.output.resolve()
    require(not output.exists() and not output.is_symlink(),'exclusive proof directory required')
    require(os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted','fresh-hosted proof requires a separate GitHub-hosted job')
    require(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()==os.environ['GITHUB_SHA'],'checkout/source mismatch')
    output.mkdir(mode=0o700);steps=[]
    def run(name,command,seconds=600,check=True):
        started=time.monotonic()
        with (output/(name+'.stdout')).open('xb') as stdout,(output/(name+'.stderr')).open('xb') as stderr:
            try:status=subprocess.run([str(c) for c in command],cwd=ROOT,stdout=stdout,stderr=stderr,timeout=seconds).returncode
            except subprocess.TimeoutExpired:status=124
        record={'name':name,'command':[str(c) for c in command],'exit_code':status,'elapsed_seconds':time.monotonic()-started,
                'scope':'correctness operation wall time; not an isolated build benchmark or daemon CPU accounting'}
        steps.append(record);write(output/(name+'.json'),record)
        if check:require(status==0,'proof command failed; retained: '+name)
        return status,(output/(name+'.stdout')).read_text()
    index_api.unpack_packet(packet,output/'packet');packet=output/'packet'
    invocation=index_api.invocation();index=read(packet/'index.json');index_api.validate(index,invocation);index_api.verify_parent(index,packet)
    for role,path in (('sdk','sdk-materialized/materialization.json'),('chef','chef-oci-integrity.json')):
        require(index_api.sha(packet/path)==index['inputs'][role]['oci_receipt_sha256'],'signed raw OCI receipt differs: '+role)
    verify=['--certificate-identity',invocation['workflow_identity'],'--certificate-oidc-issuer',invocation['oidc_issuer'],
            '--certificate-github-workflow-repository',invocation['repository'],'--certificate-github-workflow-ref',invocation['source_ref'],
            '--certificate-github-workflow-sha',invocation['source_sha']]
    run('source-clean',['git','diff','--quiet','HEAD','--'],30)
    run('source-archive',['git','archive','--format=tar','--output',output/'source.tar','HEAD'],60)
    context=output/'source-context';materials_api.extract(output/'source.tar',context)
    write(output/'source-context.json',{'source_sha':invocation['source_sha'],'archive_sha256':index_api.sha(output/'source.tar'),'inventory':audit.inventory(context)})
    run('packet-integrity',['bash','-euo','pipefail','-c','cd -- "$1"; sha256sum --check --strict SHA256SUMS','_',packet])
    run('index-signature',['cosign','verify-blob','--bundle',packet/'index.sigstore.json',*verify,packet/'index.json'])
    require(index_api.sha(packet/'production-candidates.json')==index['production_candidate_index_sha256'],'six-image index binding differs')
    run('production-index-signature',['cosign','verify-blob','--bundle',packet/'production-candidates.sigstore.json',*verify,packet/'production-candidates.json'])
    production=read(packet/'production-candidates.json')
    require(all(production.get(k)==v for k,v in invocation.items()) and sorted(r['role'] for r in production['images'])==['backend','bridge','gateway','native','tesseract','tika'],'signed original six-image invocation/set differs')
    # Before any dependency pull/build. Record runner/daemon facts and reject
    # imported build state; no cache-from/cache-to flags exist in this proof.
    _,builder=run('initial-builder',['docker','buildx','inspect'],30)
    require('Driver:        docker' in builder or any(line.split()==['Driver:','docker'] for line in builder.splitlines()),'expected default docker driver')
    _,cache=run('initial-build-cache',['docker','buildx','du','--format=json'],60);initial_cache_empty(cache)
    run('initial-docker-state',['docker','system','df','--format','{{json .}}'],30)
    write(output/'runner.json',{k:os.environ.get(k) for k in ('RUNNER_NAME','RUNNER_ENVIRONMENT','RUNNER_OS','GITHUB_RUN_ID','GITHUB_RUN_ATTEMPT','GITHUB_JOB')})
    for role in ('sdk','chef'):
        entry=index['inputs'][role];reference=entry['reference']
        require(index_api.sha(packet/(role+'.spdx.json'))==entry['spdx_sha256'],'SBOM packet binding differs')
        run(role+'-signature',['cosign','verify',*verify,reference])
        for kind,name,expected in [('spdxjson','sbom',read(packet/(role+'.spdx.json'))),
                                  ('https://github.com/somehowchris/tika-indra-ocr-client/attestations/native-build-inputs/v1','inputs',index)]:
            _,body=run(role+'-'+name+'-attestation',['cosign','verify-attestation',*verify,'--type',kind,reference])
            require_attestation(body,expected,entry['digest'])
        run(role+'-download',['crane','pull','--format=oci',reference,output/(role+'-oci')],1800)
    run('sdk-materialize',['python3',ROOT/'scripts/native-sdk.py','materialize-oci','--layout',output/'sdk-oci','--digest',index['inputs']['sdk']['digest'],'--inputs',ROOT,'--output',output/'sdk-materialized'])
    sdk=read(output/'sdk-materialized/materialization.json')
    require(sdk['recipe_sha256']==index['inputs']['sdk']['recipe_sha256'] and sdk['content_sha256']==index['inputs']['sdk']['content_sha256'],'downloaded SDK manifests differ')
    run('chef-distribution',['python3',ROOT/'scripts/native-sdk.py','verify-oci','--layout',output/'chef-oci','--digest',index['inputs']['chef']['digest'],'--output',output/'chef-oci-verification.json'],1800)
    run('render-sdk-consumer',['python3',ROOT/'scripts/native-sdk.py','render-consumer','--source',ROOT/'docker/native.Dockerfile','--output',output/'sdk.Dockerfile'])
    run('render-chef-consumer',['python3',ROOT/'scripts/render-chef-consumer.py','--source',ROOT/'docker/native.Dockerfile','--mode','consumer','--output',output/'chef.Dockerfile'])
    verify_materials(packet/'baseline/server-acceptance/relink',context,output/'sdk-materialized/sdk',sdk['recipe_sha256'],output/'baseline-materials.json')
    models=output/'models';models.mkdir(mode=0o777);models.chmod(0o777)
    token=secrets.token_hex(16);owner_label='org.tika-indra.native-build-proof'
    def container(name,image,resolution,command,cpus,network='none',mounts=(),env=()):
        directory=output/name;directory.mkdir();identifier=None
        affinity='0' if cpus==1 else '0,1';container_name='indra-build-proof-'+token+'-'+name
        create=['docker','create','--label',owner_label+'='+token,'--name',container_name,
                '--network',network,'--read-only','--cpus',str(cpus),'--cpuset-cpus',affinity,'--memory','3g','--memory-swap','3g','--pids-limit','128',
                '--cap-drop','ALL','--security-opt','no-new-privileges','--ulimit','stack=8388608:8388608',
                '--tmpfs','/tmp:rw,nosuid,nodev,noexec,size=32m,mode=1777']
        for value in mounts:create+=['--mount',value]
        for value in env:create+=['--env',value]
        if command[0].startswith('/'):
            create+=['--entrypoint',command[0]];command=command[1:]
        create+=[image,*command]
        primary=None;cleanup_errors=[]
        try:
            _,body=run(name+'-create',create,30);identifier=body.strip()
            require(len(identifier)==64 and all(c in '0123456789abcdef' for c in identifier),'container ID missing')
            _,body=run(name+'-before',['docker','inspect',identifier],30);inspection=json.loads(body)[0]
            require(inspection['Name']=='/'+container_name and inspection['Config']['Labels'].get(owner_label)==token and inspection['Id']==identifier,'container owner mismatch')
            config=inspection['HostConfig']
            require(config['NanoCpus']==cpus*10**9 and config['CpusetCpus']==affinity and config['Memory']==config['MemorySwap']==3221225472 and config['PidsLimit']==128,'actual container limits differ')
            require(config['CapDrop']==['ALL'] and not config.get('CapAdd') and not config['Privileged'] and config['ReadonlyRootfs'] and config['PidMode']=='' and config['NetworkMode']==network,'container isolation differs')
            run(name+'-bind',['bash',ROOT/'scripts/verify-daemon-image.sh','bind',resolution,output/(name+'-before.stdout'),directory/'binding'],1200)
            run(name+'-start',['docker','start','--attach',identifier],1800)
            _,body=run(name+'-final',['docker','inspect',identifier],30);final=json.loads(body)[0]
            require(final['State']['ExitCode']==0 and not final['State']['Running'] and not final['State']['OOMKilled'],'test failed/OOM')
        except BaseException as error:
            primary=type(error).__name__+': '+str(error)
            raise
        finally:
            # A timed-out docker create can succeed in the daemon without a
            # stdout ID. Recover only this unpredictable exact name; require
            # matching owner AND immutable image before any cleanup mutation.
            try:
                status,body=run(name+'-cleanup-recover',['docker','inspect',container_name],30,False)
                if status==0:
                    record=json.loads(body)[0];identifier=owned_container(record,container_name,token,image)
                    run(name+'-cleanup-bind',['bash',ROOT/'scripts/verify-daemon-image.sh','bind',resolution,output/(name+'-cleanup-recover.stdout'),directory/'cleanup-binding'],1200)
                    if record['State']['Running']:run(name+'-stop',['docker','stop','--timeout','15',identifier],45)
                    run(name+'-remove',['docker','rm',identifier],30)
                else:
                    require(status!=124 and 'No such' in (output/(name+'-cleanup-recover.stderr')).read_text(),'cleanup recovery did not prove container absence')
                status,remaining=run(name+'-cleanup-absence',['docker','ps','--all','--quiet','--filter','name=^/'+container_name+'$'],30,False)
                require(status==0 and not remaining.strip(),'exact owned container remains after cleanup')
            except BaseException as error:cleanup_errors.append(type(error).__name__+': '+str(error))
            write(directory/'cleanup.json',{'schema_version':1,'name':container_name,'owner':token,'image':image,'primary_failure':primary,'cleanup_errors':cleanup_errors,'passed':not cleanup_errors})
            if cleanup_errors and primary is None:raise ValueError('container cleanup failed: '+name)
    try:
        for arm in ('sdk','chef'):
            arm_directory=output/arm;arm_directory.mkdir()
            common=['docker','buildx','build','--platform','linux/amd64','--file',output/(arm+'.Dockerfile'),
                    '--build-context',f'sdk-input=oci-layout://{output}/sdk-oci@{index["inputs"]["sdk"]["digest"]}',
                    '--build-arg','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'--build-arg','NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256'],
                    '--build-arg','BUILD_CPUSET=0,1','--provenance=false','--sbom=false']
            if arm=='chef':common+=['--build-context',f'chef-input=oci-layout://{output}/chef-oci@{index["inputs"]["chef"]["digest"]}',
                                   '--build-arg','CHEF_DEPENDENCY_KEY_SHA256='+index['inputs']['chef']['recipe_sha256'],
                                   '--build-arg','CHEF_CONTENT_SHA256='+index['inputs']['chef']['content_sha256']]
            images={};resolutions={}
            for target in ('engine-acceptance','server-acceptance'):
                run(arm+'-'+target+'-export',[*common,'--target',target,'--metadata-file',arm_directory/(target+'.metadata.json'),'--output','type=local,dest='+str(arm_directory/target),context],7200)
                tag='indra-build-proof-'+arm+'-'+target+':local'
                run(arm+'-'+target+'-load',[*common,'--target',target,'--load','--tag',tag,context],600)
                resolution=arm_directory/(target+'-resolution')
                run(arm+'-'+target+'-resolve',['bash',ROOT/'scripts/verify-daemon-image.sh','resolve',tag,resolution],1200)
                images[target]=read(resolution/'result.json')['root_digest'];resolutions[target]=resolution
            binaries={}
            for role,(target,relative) in index_api.ROLES.items():
                path=arm_directory/target/relative;expected=index['baseline_roles'][role]
                require(path.stat().st_size==expected['bytes'] and index_api.sha(path)==expected['sha256'],'seven-role executable differs: '+arm+'/'+role)
                binaries[role]=audit.static_elf(path)
            write(arm_directory/'seven-role-static.json',binaries)
            baseline=packet/'baseline/server-acceptance/relink';actual=arm_directory/'server-acceptance/relink'
            verify_materials(actual,context,output/'sdk-materialized/sdk',sdk['recipe_sha256'],arm_directory/'materials.json')
            require(audit.inventory(baseline)==audit.inventory(actual),'complete source/vendor/native/notices materials differ')
            if arm=='sdk':container('model-acquisition',images['engine-acceptance'],resolutions['engine-acceptance'],['models'],1,'bridge',[f'type=bind,src={models},dst=/models'])
            for cpus in (1,2):
                destination=arm_directory/f'cpu-{cpus}';destination.mkdir(mode=0o777);destination.chmod(0o777)
                container(f'{arm}-engine-{cpus}',images['engine-acceptance'],resolutions['engine-acceptance'],['test','--threads',str(cpus)],cpus,mounts=[f'type=bind,src={models},dst=/models,readonly',f'type=bind,src={destination},dst=/evidence'])
                for gate in ('server-unit','server-native'):
                    container(f'{arm}-{gate}-{cpus}',images['server-acceptance'],resolutions['server-acceptance'],['/gates/'+gate,'--include-ignored','--test-threads=1','--nocapture'],cpus,mounts=[f'type=bind,src={models},dst=/models,readonly'],env=['INDRA_TEST_MODEL_DIR=/models','INDRA_TEST_SERVER_BIN=/indra-server','INDRA_TEST_THREADS='+str(cpus),'RUST_MIN_STACK=8388608'])
        # Invalidation and tiny C/Rust archive experiment are independent gates;
        # they must finish before this job can report the full build-input proof.
        run('sdk-invalidation',['python3',ROOT/'scripts/verify-sdk-invalidation.py','--index',packet/'index.json','--inputs',output,'--output',output/'sdk-invalidation'],7200)
        run('chef-invalidation',['python3',ROOT/'scripts/verify-chef-invalidation.py','--index',packet/'index.json','--inputs',output,'--output',output/'invalidation'],7200)
    finally:
        primary=sys.exc_info()[1]
        status,remaining=run('final-owned-containers',['docker','ps','--all','--quiet','--filter',owner_label+'='+token],30,False)
        write(output/'final-cleanup.json',{'passed':status==0 and not remaining.strip(),'status':status,'remaining':remaining.splitlines(),'primary_failure':str(primary) if primary else None})
        require(status==0 and not remaining.strip(),'owned containers remain after correctness gates')
    write(output/'result.json',{'schema_version':1,'passed':True,'source_sha':invocation['source_sha'],'runner':'separate github-hosted job with empty initial build cache and no cache imports',
        'arms':['sdk','chef'],'roles_per_arm':7,'cpu_caps':[1,2],'source_materials_equal':True,'required_source_files_match_actual_context':True,'exact_31_native_notices_match_sdk':True,'sdk_invalidation_passed':True,'chef_invalidation_passed':True,'production_candidate_images':6,'performance_claim':False})


if __name__=='__main__':main()
