#!/usr/bin/env python3
"""Frozen WP1 seven-role content proof on one shared builder; no native fallback."""
from __future__ import annotations
import argparse
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile

ROOT=Path(__file__).resolve().parent.parent
PLAN=ROOT/'evaluation/build-cost/evidence/seven-role-plan-2026-10-18.json'
AMENDMENT=ROOT/'evaluation/build-cost/evidence/seven-role-epoch0-amendment-2026-10-19.json'
BASELINE='2193324bf42984c9dd3009fd4226b96f2379fad3'
CANDIDATE='29695e83e2f22776b8d05d58980854c7a28e9dcb'
HELPER='2a16bb3b39bd14dbf30fae62ee58ead931e0fcd1'
ROLES={'indra-server':('server-acceptance','indra-server'),'gates/server-unit':('server-acceptance','gates/server-unit'),
       'gates/server-native':('server-acceptance','gates/server-native'),'ocr':('engine-acceptance','ocr'),
       'engine-acceptance':('engine-acceptance','engine-acceptance'),'gates/unit':('engine-acceptance','gates/unit'),
       'gates/native':('engine-acceptance','gates/native')}
DELTA={'docker/native.Dockerfile','scripts/select-cargo-artifact.py'}
SOURCE_ROOTS=('Cargo.toml','Cargo.lock','src','tests','benches','runtime','shared','backend/models.lock.json',
              'backend/tests/data','scripts','docker','docs','licenses','LICENSE','THIRD_PARTY_NOTICES.md')


def require(value,message):
    if not value:raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):return json.loads(path.read_text())


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False);stream.write('\n')


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


def controller_inputs():
    return {'scripts/prove-frozen-wp1.py':sha(Path(__file__)),
            'evaluation/build-cost/audit.py':sha(ROOT/'evaluation/build-cost/audit.py'),
            'evaluation/build-cost/evidence/seven-role-plan-2026-10-18.json':sha(PLAN),
            'evaluation/build-cost/evidence/seven-role-epoch0-amendment-2026-10-19.json':sha(AMENDMENT)}


def verify_source_binding(revision,inputs):
    # Later unrelated commits are permitted; these exact tool bytes must belong
    # to the recorded producing commit, rather than a dirty current checkout.
    require(isinstance(revision,str) and re.fullmatch(r'[0-9a-f]{40}',revision) is not None,'invalid controller producing source')
    for name,digest in inputs.items():
        original=subprocess.check_output(['git','show',revision+':'+name],cwd=ROOT,timeout=30)
        require(hashlib.sha256(original).hexdigest()==digest,'controller input does not match producing source: '+name)


APPLICATION_CLOCK_STAGE='# Reproducibility amendment: fixed C dependency date/time, not actual build time.\nFROM native-build AS application-build\nENV SOURCE_DATE_EPOCH=0\n\n'


def amend_application_clock(original):
    marker='# Application compilation is cached independently of the expensive archives.\n'
    require(original.count(marker)==1 and 'SOURCE_DATE_EPOCH' not in original,'original application clock recipe differs')
    before,after=original.split(marker)
    for stage in ('engine-build','server-build','engine-profile-build'):
        source='FROM native-build AS '+stage+'\n'
        require(after.count(source)==1,'original application stage missing or ambiguous: '+stage)
        after=after.replace(source,'FROM application-build AS '+stage+'\n')
    return before+marker+APPLICATION_CLOCK_STAGE+after


def verify_execution_inputs(output,prepared):
    identity=read(output/'controller-identity.json')
    actual=controller_inputs()
    require(identity['controller_sha256']==actual['scripts/prove-frozen-wp1.py'],'controller changed after preparation')
    require(identity['audit_sha256']==actual['evaluation/build-cost/audit.py'],'audit changed after preparation')
    require(identity['plan_sha256']==prepared['plan_sha256']==sha(PLAN)==sha(output/'plan.json'),'frozen plan identity differs after preparation')
    verify_source_binding(identity['controller_source_sha'],actual)
    if prepared.get('application_epoch_zero'):
        require(prepared['amendment_sha256']==sha(AMENDMENT)==sha(output/'amendment.json'),'predeclared amendment changed after preparation')
        for arm in ('baseline','candidate'):
            original=output/(arm+'.original-rendered.Dockerfile')
            require(sha(original)==prepared['original_rendered_dockerfile_sha256'][arm], 'original rendered recipe changed after preparation')
            require((output/(arm+'.Dockerfile')).read_text()==amend_application_clock(original.read_text()),'rendered recipe differs outside declared common application amendment')
    require(prepared['source_archive_sha256']==sha(output/'baseline-source.tar'),'retained baseline source archive changed after preparation')
    for arm in ('baseline','candidate'):
        require(audit_module().inventory(output/'contexts'/arm)==read(output/(arm+'-context.json')),'context changed after preparation')
        require(sha(output/(arm+'.Dockerfile'))==prepared['rendered_dockerfile_sha256'][arm],'rendered immutable consumer recipe changed after preparation')
    return identity


_audit=None


def audit_module():
    global _audit
    if _audit is None:
        _audit=module('frozen_wp1_audit',ROOT/'evaluation/build-cost/audit.py')
    return _audit


def extract(archive,destination):
    """Admit every member before extracting any byte; preserve ordinary modes."""
    require(not destination.exists() and not destination.is_symlink(),'exclusive extraction directory required')
    with tarfile.open(archive,'r:*') as tar:
        members=tar.getmembers();require(len(members)<=200000,'source member bound exceeded')
        seen=set();total=0
        for member in members:
            path=Path(member.name)
            logical=path.as_posix()
            require(path.parts and not path.is_absolute() and '..' not in path.parts and '\\' not in member.name and member.name in (logical,logical+'/') and logical not in seen,'unsafe/duplicate source member')
            require(member.isdir() or member.isfile(),'source link/special file forbidden')
            require(member.mode&0o7000==0,'source special mode forbidden')
            total+=member.size;require(total<=4294967296,'source uncompressed bytes exceed4GiB')
            seen.add(logical)
        destination.mkdir(mode=0o700)
        def admitted(member,directory):
            return tarfile.data_filter(member,directory).replace(mode=member.mode&0o777,uid=None,gid=None,uname=None,gname=None)
        tar.extractall(destination,members=members,filter=admitted)


def difference(baseline,candidate):
    before={record['path']:record for record in baseline};after={record['path']:record for record in candidate}
    return [{'path':path,'baseline':before.get(path),'candidate':after.get(path)} for path in sorted(before.keys()|after.keys()) if before.get(path)!=after.get(path)]


def verify_manifest(directory):
    """Listed hash validity and complete listed file inventory are separate gates."""
    names=[]
    for line in (directory/'SHA256SUMS').read_text().splitlines():
        match=re.fullmatch(r'([0-9a-f]{64})  (\./[^\n]+)',line)
        require(match is not None,'invalid material checksum record')
        name=match[2][2:];path=Path(name)
        require(path.parts and not path.is_absolute() and '..' not in path.parts and '\\' not in name and path.as_posix()==name and name not in names,'unsafe/duplicate checksum name')
        actual=directory/path;require(actual.is_file() and not actual.is_symlink() and sha(actual)==match[1],'material checksum differs: '+name)
        names.append(name)
    actual={record['path'] for record in audit_module().inventory(directory) if record['type']=='file' and record['path']!='SHA256SUMS'}
    require(set(names)==actual,'material checksum inventory incomplete or extra')


def verify_source_completeness(context,source):
    # Required originals come from the frozen context, not from the other arm or
    # from a self-declared checksum list inside the produced archive.
    for name in SOURCE_ROOTS:
        require((context/name).is_file() or (context/name).is_dir(),'required frozen context root missing: '+name)
    required={}
    for record in audit_module().inventory(context):
        name=record['path'];parts=Path(name).parts
        selected=any(name==root or name.startswith(root+'/') for root in SOURCE_ROOTS)
        excluded='target' in parts or name=='runtime/proofs' or name.startswith('runtime/proofs/')
        if record['type']=='file' and selected and not excluded:required[name]=record
    actual={record['path']:record for record in audit_module().inventory(source) if record['type']=='file'}
    for name,record in required.items():
        require(name in actual and actual[name]['bytes']==record['bytes'] and actual[name]['sha256']==record['sha256'],
                'required source material missing or changed: '+name)
    require('.cargo/config.toml' in actual and actual['.cargo/config.toml']['bytes']>0,'generated vendor configuration missing')
    require((source/'vendor').is_dir() and any(name.startswith('vendor/') for name in actual),'generated vendor source tree empty')
    additional=sorted(set(actual)-set(required))
    require(all(name.startswith(('vendor/','.cargo/')) for name in additional),'unexpected generated source archive file')
    return {'required_context_files':[required[name] for name in sorted(required)],
            'additional_generated_files':[actual[name] for name in additional],
            'required_source_files_match_context':True,'generated_configuration_and_vendor_present':True,
            'excluded_context_scope':'Files outside declared collector roots; all target components and runtime/proofs are deliberately excluded.'}


def verify_native_notices(materials,sdk,collector,expected_recipe_sha256):
    require(sha(sdk/'recipe.json')==expected_recipe_sha256,'verified SDK recipe changed before notice admission')
    recipe=read(sdk/'recipe.json');expected=recipe['notice_paths']
    require(len(expected)==31 and len(set(expected))==31,'verified SDK notice path set differs')
    require(sha(collector)==recipe['source']['source_files']['scripts/collect-runtime-materials.sh'],
            'notice collector differs from verified SDK recipe')
    text=collector.read_text();marker="done <<'NATIVE_NOTICES'\n"
    require(text.count(marker)==1,'notice collector delimiter differs')
    body=text.split(marker)[1].split('\nNATIVE_NOTICES\n')[0]
    mapping={}
    for line in body.splitlines():
        component,name=line.split('|');relative=Path(name)
        require(name not in mapping and relative.parts and not relative.is_absolute() and '..' not in relative.parts
                and relative.as_posix()==name and re.fullmatch(r'[a-z0-9-]+',component) is not None,'invalid expected notice mapping')
        mapping[name]=(component,component+'/'+relative.name)
    require(set(mapping)==set(expected),'collector paths differ from verified SDK notice recipe')
    notice_root=materials/'licenses/native'
    with (notice_root/'inventory.tsv').open(newline='') as stream:
        rows=list(csv.DictReader(stream,delimiter='\t'))
    keys={'component','source_relative_path','notice_relative_path','bytes','sha256'}
    require(len(rows)==31 and all(set(row)==keys for row in rows),'native notice inventory schema/count differs')
    require({row['source_relative_path'] for row in rows}==set(expected),'native notice source path set differs')
    destinations=set();verified=[]
    for row in rows:
        name=row['source_relative_path'];component,destination=mapping[name]
        require(row['component']==component and row['notice_relative_path']==destination and destination not in destinations,
                'native notice destination/component differs: '+name)
        require(re.fullmatch(r'[0-9]+',row['bytes']) is not None and re.fullmatch(r'[0-9a-f]{64}',row['sha256']) is not None,
                'native notice size/hash invalid: '+name)
        original=sdk/'rootfs/native'/name;copied=notice_root/destination
        for path in (original,copied):
            require(path.is_file() and not path.is_symlink() and path.stat().st_size==int(row['bytes']) and sha(path)==row['sha256'],
                    'native notice bytes differ from verified SDK source: '+name)
        destinations.add(destination);verified.append(dict(row))
    actual={record['path'] for record in audit_module().inventory(notice_root) if record['type']=='file'}
    require(actual==destinations|{'inventory.tsv','README.txt'},'native notice file inventory incomplete or extra')
    return {'verified_notices':verified,'expected_recipe_sha256':expected_recipe_sha256,
            'source_paths_and_actual_sdk_bytes_and_destinations_verified':True}


def prepare(output,sdk_layout,run,application_epoch_zero=False):
    plan=read(PLAN)
    require(plan['baseline_application_source']==BASELINE and plan['candidate_recipe_source']==CANDIDATE and plan['sdk_helper_source']==HELPER,'frozen WP1 source identities differ')
    require(plan['targets_per_arm']==['engine-acceptance','server-acceptance'] and set(plan['roles'])==set(ROLES),'frozen target/role set differs')
    require(plan['sdk']=={'sdk_platform_manifest':'sha256:3a98d2e4588bfb13fbca7caac07ba8caa060d4af07a362f5efa21557a4cbe14a','recipe_sha256':'d532e54f547ca606d2df1d546fb3ef94c372160efda4bbd12877435b7d0e8bc7','content_sha256':'b4810a9984dcb21493cc330e2f08b28849429d5c3cb055cff29e0733356b5411'},'frozen native SDK identity differs')
    (output/'plan.json').write_bytes(PLAN.read_bytes())
    if application_epoch_zero:
        amendment=read(AMENDMENT)
        require(amendment['application_environment']=={'SOURCE_DATE_EPOCH':'0'} and amendment['arms']==['baseline','candidate','baseline-replicate'] and amendment['no_cache_filter']==['engine-build','server-build'],'predeclared clock amendment differs')
        (output/'amendment.json').write_bytes(AMENDMENT.read_bytes())
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    inputs=controller_inputs();verify_source_binding(revision,inputs)
    write(output/'controller-identity.json',{'controller_source_sha':revision,
        'controller_sha256':inputs['scripts/prove-frozen-wp1.py'],'audit_sha256':inputs['evaluation/build-cost/audit.py'],'plan_sha256':sha(PLAN),
        'scope':'immutable source/content proof; shared builder, no fresh-runner, signing, lifecycle or speedup admission'})
    archive=output/'baseline-source.tar'
    with archive.open('xb') as stream:subprocess.run(['git','archive',BASELINE],cwd=ROOT,stdout=stream,check=True,timeout=60)
    contexts=output/'contexts';contexts.mkdir()
    for arm in ('baseline','candidate'):
        context=contexts/arm;extract(archive,context)
        additions={'scripts/native-sdk.py':HELPER}
        if arm=='candidate':additions.update({name:CANDIDATE for name in DELTA})
        for name,source in additions.items():(context/name).write_bytes(subprocess.check_output(['git','show',source+':'+name],cwd=ROOT,timeout=30))
        write(output/(arm+'-context.json'),audit_module().inventory(context))
        require(sha(context/'scripts/native-sdk.py')==plan['sdk_input_recipe_files_verified_against_baseline_and_helper_source']['source_files']['scripts/native-sdk.py'],'common SDK helper differs')
        run('render-'+arm,['python3',context/'scripts/native-sdk.py','render-consumer','--source',context/'docker/native.Dockerfile','--output',output/(arm+'.Dockerfile')],60)
        if application_epoch_zero:
            original=(output/(arm+'.Dockerfile')).read_text()
            (output/(arm+'.original-rendered.Dockerfile')).write_text(original)
            (output/(arm+'.Dockerfile')).write_text(amend_application_clock(original))
        prefix=(output/(arm+'.Dockerfile')).read_text().split('# Application compilation is cached independently',1)[0]
        require('build-native-runtime.sh /native' not in prefix and 'FROM sdk-input AS native-sdk-input' in prefix,'native source-build fallback present')
    delta=difference(read(output/'baseline-context.json'),read(output/'candidate-context.json'))
    write(output/'context-difference.json',delta)
    require({row['path'] for row in delta}==DELTA,'source context changed outside declared Dockerfile/selector delta')
    sdk=plan['sdk'];run('sdk-materialize',['python3',contexts/'baseline/scripts/native-sdk.py','materialize-oci','--layout',sdk_layout,'--digest',sdk['sdk_platform_manifest'],'--inputs',contexts/'baseline','--output',output/'sdk-materialized'],1800)
    actual=read(output/'sdk-materialized/materialization.json')
    require(actual['platform_manifest_digest']==sdk['sdk_platform_manifest'] and actual['recipe_sha256']==sdk['recipe_sha256'] and actual['content_sha256']==sdk['content_sha256'],'materialized native SDK differs')
    write(output/'prepared.json',{'schema_version':1,'prepared':True,'sdk_layout':str(sdk_layout),'plan_sha256':sha(PLAN),'source_archive_sha256':sha(archive),'rendered_dockerfile_sha256':{arm:sha(output/(arm+'.Dockerfile')) for arm in ('baseline','candidate')},'application_epoch_zero':application_epoch_zero,'amendment_sha256':sha(AMENDMENT) if application_epoch_zero else None,'original_rendered_dockerfile_sha256':{arm:sha(output/(arm+'.original-rendered.Dockerfile')) for arm in ('baseline','candidate')} if application_epoch_zero else None,'native_source_fallback':False,'context_modes':'original git archive, with identical common helper write; no mode normalization'})


def compare(output):
    roles={};equal=True
    for role,(target,name) in ROLES.items():
        sides={arm:audit_module().static_elf(output/arm/target/name) for arm in ('baseline','candidate')}
        matched=sides['baseline']['sha256']==sides['candidate']['sha256'];equal=equal and matched
        roles[role]={**sides,'equal':matched}
    write(output/'seven-role-comparison.json',roles)
    require(equal,'seven-role executable byte identity differs; inspect retained comparison before proceeding')
    paths={arm:output/arm/'server-acceptance/relink' for arm in ('baseline','candidate')}
    outer={};inner={};source_admission={};notice_admission={}
    for arm,path in paths.items():
        verify_manifest(path)
        outer[arm]=audit_module().inventory(path);write(output/(arm+'-materials.json'),outer[arm])
        source=output/(arm+'-source-materials');extract(path/'rust-source.tar.gz',source)
        inner[arm]=audit_module().inventory(source);write(output/(arm+'-source-materials.json'),inner[arm])
        source_admission[arm]=verify_source_completeness(output/'contexts'/arm,source)
        notice_admission[arm]=verify_native_notices(path,output/'sdk-materialized/sdk',
            output/'contexts'/arm/'scripts/collect-runtime-materials.sh',read(output/'plan.json')['sdk']['recipe_sha256'])
    write(output/'source-completeness.json',source_admission)
    write(output/'native-notice-admission.json',notice_admission)
    outer_delta=difference(outer['baseline'],outer['candidate']);inner_delta=difference(inner['baseline'],inner['candidate'])
    write(output/'material-difference.json',{'outer':outer_delta,'extracted_source':inner_delta})
    require({row['path'] for row in outer_delta}=={'SHA256SUMS','rust-source.tar.gz'},'unexplained native/vendor/notices or outer material difference')
    require({row['path'] for row in inner_delta}==DELTA,'source/vendor materials differ outside declared Dockerfile/selector delta')
    for arm in ('baseline','candidate'):
        context=output/'contexts'/arm;source=output/(arm+'-source-materials')
        for name in DELTA:
            expected=context/name;actual=source/name
            require((not expected.exists() and not actual.exists()) or (expected.is_file() and actual.is_file() and sha(expected)==sha(actual)),'source archive does not contain actual declared recipe/selector bytes')
    for name in ('build-evidence/default-config-check.txt','build-evidence/openapi.json'):
        before=output/'baseline/server-acceptance'/name;after=output/'candidate/server-acceptance'/name
        require(before.read_bytes()==after.read_bytes(),'runtime configuration/OpenAPI output differs')
    write(output/'comparison.json',{'schema_version':1,'passed':True,'seven_binary_hashes_equal':True,'static_elf_roles':7,
        'native_sdk_content_equal':True,'source_materials_differences':sorted(DELTA),'all_other_materials_equal':True,
        'runtime_config_openapi_equal':True,'required_source_files_verified_against_each_context':True,
        'all_31_notice_sources_and_sdk_bytes_verified':True,'lifecycle_executed':False,'fresh_runner_proof':False,'signing_admission':False,'performance_claim':False})


def compare_baseline_replicate(output):
    roles={}
    for role,(target,name) in ROLES.items():
        before=audit_module().static_elf(output/'baseline'/target/name)
        after=audit_module().static_elf(output/'baseline-replicate'/target/name)
        roles[role]={'baseline':before,'baseline_replicate':after,'equal':before['sha256']==after['sha256']}
    write(output/'baseline-replicate-seven-roles.json',roles)
    require(all(row['equal'] for row in roles.values()),'independent same-source baseline seven-role byte identity differs')
    materials=output/'baseline-replicate/server-acceptance/relink'
    verify_manifest(materials)
    require(audit_module().inventory(materials)==read(output/'baseline-materials.json'),'independent same-source baseline material inventory differs')
    source=output/'baseline-replicate-source-materials';extract(materials/'rust-source.tar.gz',source)
    require(audit_module().inventory(source)==read(output/'baseline-source-materials.json'),'independent same-source baseline extracted source/vendor inventory differs')
    completeness=verify_source_completeness(output/'contexts/baseline',source)
    notices=verify_native_notices(materials,output/'sdk-materialized/sdk',output/'contexts/baseline/scripts/collect-runtime-materials.sh',read(output/'plan.json')['sdk']['recipe_sha256'])
    for name in ('build-evidence/default-config-check.txt','build-evidence/openapi.json'):
        require((output/'baseline/server-acceptance'/name).read_bytes()==(output/'baseline-replicate/server-acceptance'/name).read_bytes(),'independent same-source baseline configuration/OpenAPI output differs')
    write(output/'baseline-replicate-comparison.json',{'passed':True,'seven_raw_executable_hashes_equal':True,'full_material_inventory_equal':True,'source_completeness':completeness,'native_notices':notices,'configuration_openapi_equal':True,'application_environment':{'SOURCE_DATE_EPOCH':'0'},'output_normalization':False})


def main():
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='action',required=True)
    prepare_parser=sub.add_parser('prepare');prepare_parser.add_argument('--sdk-layout',type=Path,required=True);prepare_parser.add_argument('--output',type=Path,required=True);prepare_parser.add_argument('--application-epoch-zero',action='store_true',help='apply predeclared common C-clock amendment and require independent baseline replication')
    run_parser=sub.add_parser('run');run_parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();output=args.output.resolve();require(not args.output.is_symlink(),'symlink output rejected')
    if args.action=='prepare':require(not output.exists(),'exclusive proof directory required');output.mkdir(mode=0o700)
    else:require(output.is_dir() and not (output/'execution.json').exists(),'prepared unexecuted proof directory required')
    def run(name,command,seconds):
        subprocess.run(['python3',ROOT/'evaluation/build-cost/audit.py','record-phase','--phase',name,'--output',output/(name+'.phase'),'--','timeout','--signal=TERM','--kill-after=15s',str(seconds)+'s',*[str(v) for v in command]],cwd=ROOT,check=True)
    if args.action=='prepare':
        try:prepare(output,args.sdk_layout.resolve(),run,args.application_epoch_zero)
        finally:
            error=sys.exc_info()[1]
            write(output/'preparation-completion.json',{'schema_version':1,'passed':error is None,'failure':str(error) if error else None,'builds_executed':False,'performance_claim':False})
        return
    prepared=read(output/'prepared.json');plan=read(output/'plan.json');sdk=plan['sdk'];sdk_layout=Path(prepared['sdk_layout'])
    verify_execution_inputs(output,prepared)
    write(output/'execution.json',{'schema_version':1,'builder_scope':plan['builder_scope'],'compiler_jobs':2,'compiler_affinity_admitted':False,
        'native_build_cpuset_argument':'0,1','resource_scope':'original compiler commands preserved; BUILD_CPUSET does not impose Rust compiler affinity/quota; client wait4 does not measure BuildKit daemon CPU',
        'lifecycle_executed':False,'fresh_builder_claim':False,'performance_claim':False,'application_environment':{'SOURCE_DATE_EPOCH':'0'} if prepared.get('application_epoch_zero') else None,'no_cache_filter':['engine-build','server-build'] if prepared.get('application_epoch_zero') else []})
    try:
        run('builder-observation',['docker','buildx','inspect'],30)
        arms=('baseline','candidate','baseline-replicate') if prepared.get('application_epoch_zero') else ('baseline','candidate')
        for arm in arms:
            source_arm='baseline' if arm=='baseline-replicate' else arm
            context=output/'contexts'/source_arm
            (output/arm).mkdir()
            for target in plan['targets_per_arm']:
                command=['docker','buildx','build','--platform','linux/amd64','--progress=plain','--file',output/(source_arm+'.Dockerfile'),
                    '--build-context',f'sdk-input=oci-layout://{sdk_layout}@{sdk["sdk_platform_manifest"]}',
                    '--build-arg','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'--build-arg','NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256'],
                    '--build-arg','BUILD_CPUSET=0,1','--provenance=false','--sbom=false','--target',target,
                    '--metadata-file',output/arm/(target+'.metadata.json'),'--output','type=local,dest='+str(output/arm/target),context]
                if prepared.get('application_epoch_zero'):command[3:3]=['--no-cache-filter','engine-build,server-build']
                run(arm+'-'+target,command,7200)
        compare(output)
        if prepared.get('application_epoch_zero'):compare_baseline_replicate(output)
    finally:
        error=sys.exc_info()[1]
        write(output/'completion.json',{'schema_version':1,'passed':error is None,'failure':str(error) if error else None,'performance_claim':False,'fresh_builder_claim':False,'lifecycle_executed':False})


if __name__=='__main__':main()
