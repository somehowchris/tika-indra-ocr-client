#!/usr/bin/env python3
"""Three prespecified paired SDK-versus-Chef shared-builder repetitions per scenario."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import math
import os
import platform
from pathlib import Path
import shutil
import statistics
import subprocess
import tarfile

ROOT=Path(__file__).resolve().parent.parent


def require(value,message):
    if not value:raise ValueError(message)


def write(path,value):
    with path.open('x') as f:json.dump(value,f,indent=2,sort_keys=True,allow_nan=False);f.write('\n')


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


index_api=module('native_build_index',ROOT/'scripts/native-build-index.py')
audit=module('build_cost_audit',ROOT/'evaluation/build-cost/audit.py')
proof=module('native_build_materials_proof',ROOT/'scripts/native-build-proof.py')


def schedule():
    return [{'scenario':scenario,'repetition':rep,'order':['sdk','chef'] if rep%2==1 else ['chef','sdk']}
            for scenario in ('warm','source-comment','material-doc') for rep in (1,2,3)]


def summary(values):
    require(len(values)==3 and all(type(v) in (int,float) and math.isfinite(v) and v>=0 for v in values),'exactly three finite nonnegative repetitions required')
    return {'n':3,'min_seconds':min(values),'median_seconds':statistics.median(values),'max_seconds':max(values)}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('index','inputs','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();inputs=args.inputs.resolve();output=args.output.resolve();index=json.loads(args.index.read_text())
    require(json.loads((inputs/'result.json').read_text())['passed'] is True,'fresh correctness proof must pass before cost schedule')
    require(not output.exists() and not output.is_symlink(),'exclusive cost output required');output.mkdir(mode=0o700)
    protocol={'schema_version':1,'source_sha':index['source_sha'],'inputs':index['inputs'],'schedule':schedule(),
              'targets':['engine-acceptance','server-acceptance'],'compiler_jobs':2,'native_build_cpuset_argument':'0,1',
              'compiler_resource_admission':'not performed: preserved original Cargo stages set jobs=2 but impose no compiler CPU quota or affinity; native BUILD_CPUSET does not constrain those stages',
              'cache_state':'shared default docker builder after successful fresh hosted correctness/invalidation; no cache import/export flags',
              'arm_definitions':{'sdk':'original application stages after verified native SDK restore; dependencies compile from actual source','chef':'identical stages after verified cooked dependency restore'},
              'timed_scope':'sequential whole engine and server builds including local output export; client wait4 accounting excludes daemon CPU',
              'transport_scope':'first fresh-job crane download and SDK materialization retained separately, excluded from warm repeated builds; producer cook/export/signing retained separately and must be included in amortized cost',
              'interpretation':'descriptive min/median/max and paired whole-build differences only; no p95, causal hardware claim, or original source-native speedup claim'}
    write(output/'protocol.json',protocol)
    write(output/'protocol-identity.json',{'sha256':index_api.sha(output/'protocol.json')})
    # Host observations are context, not proof of BuildKit worker or compiler
    # affinity. Keeping original compiler commands preserves this experiment's
    # architecture comparison; runtime acceptance has separate admitted caps.
    write(output/'runner-observation.json',{'system':platform.system(),'machine':platform.machine(),
        'logical_cpus':os.cpu_count(),'controller_allowed_cpus':sorted(os.sched_getaffinity(0)),
        'scope':'controller process observation only; compiler/BuildKit limits are not admitted'})
    with (output/'docker-resource-observation.json').open('xb') as stdout,(output/'docker-resource-observation.stderr').open('xb') as stderr:
        subprocess.run(['docker','info','--format','{"logical_cpus":{{json .NCPU}},"memory_bytes":{{json .MemTotal}},"cgroup_driver":{{json .CgroupDriver}},"cgroup_version":{{json .CgroupVersion}}}'],
                       check=True,stdout=stdout,stderr=stderr,timeout=30)
    transport={}
    for name in ('sdk-download','chef-download','sdk-materialize','chef-distribution'):
        path=inputs/(name+'.json');transport[name]={'receipt':json.loads(path.read_text()),'sha256':index_api.sha(path)}
    write(output/'transport.json',transport)
    archive=output/'source.tar'
    with archive.open('xb') as f:subprocess.run(['git','-c','tar.umask=0022','archive','HEAD'],cwd=ROOT,stdout=f,check=True)
    contexts=output/'contexts';contexts.mkdir();context=contexts/'source';context.mkdir()
    with tarfile.open(archive) as tar:tar.extractall(context,filter='data')
    sdk=json.loads((inputs/'sdk-materialized/materialization.json').read_text());records=[]
    source=context/'runtime/crates/indra-engine/src/lib.rs';documentation=context/'docs/SETUP.md'
    if not documentation.exists():documentation=next(iter(sorted((context/'docs').glob('*.md'))))
    originals={source:source.read_bytes(),documentation:documentation.read_bytes()}
    for cell in protocol['schedule']:
        for path,body in originals.items():path.write_bytes(body)
        if cell['scenario']=='source-comment':source.write_bytes(originals[source]+f'\n// Chef cost repetition {cell["repetition"]}\n'.encode())
        if cell['scenario']=='material-doc':documentation.write_bytes(originals[documentation]+f'\n<!-- Chef cost repetition {cell["repetition"]} -->\n'.encode())
        pair=output/(cell['scenario']+'-'+str(cell['repetition']));pair.mkdir()
        write(pair/'context.json',{'changes':[{'path':str(path.relative_to(context)),'sha256':index_api.sha(path)} for path in originals],'source_sha':index['source_sha']})
        for arm in cell['order']:
            armout=pair/arm;armout.mkdir();elapsed=0
            common=['docker','buildx','build','--platform','linux/amd64','--progress=plain','--file',inputs/(arm+'.Dockerfile'),
                    '--build-context',f'sdk-input=oci-layout://{inputs}/sdk-oci@{index["inputs"]["sdk"]["digest"]}',
                    '--build-arg','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'--build-arg','NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256'],
                    '--build-arg','BUILD_CPUSET=0,1','--provenance=false','--sbom=false']
            if arm=='chef':common+=['--build-context',f'chef-input=oci-layout://{inputs}/chef-oci@{index["inputs"]["chef"]["digest"]}',
                                   '--build-arg','CHEF_DEPENDENCY_KEY_SHA256='+index['inputs']['chef']['recipe_sha256'],'--build-arg','CHEF_CONTENT_SHA256='+index['inputs']['chef']['content_sha256']]
            identities={}
            for target in protocol['targets']:
                destination=armout/(target+'-export');phase=armout/target
                command=[*common,'--target',target,'--metadata-file',armout/(target+'.metadata.json'),'--output','type=local,dest='+str(destination),context]
                subprocess.run(['python3',ROOT/'evaluation/build-cost/audit.py','record-phase','--phase',arm+'-'+target,'--output',phase,'--','timeout','--signal=TERM','--kill-after=15s','3600s',*[str(v) for v in command]],check=True,cwd=ROOT)
                receipt=json.loads((phase/'phase.json').read_text());require(receipt['exit_code']==0,'timed build failed');elapsed+=receipt['elapsed_ns']/1e9
                write(armout/(target+'.vertices.json'),audit.parse_log(phase/'stderr'))
                for role,(role_target,relative) in index_api.ROLES.items():
                    if role_target!=target:continue
                    path=destination/relative;actual=audit.static_elf(path);expected=index['baseline_roles'][role]
                    require(actual['sha256']==expected['sha256'] and actual['bytes']==expected['bytes'],'cost-cell executable differs: '+role)
                    identities[role]=actual
                if target=='server-acceptance':
                    proof.verify_materials(destination/'relink',context,inputs/'sdk-materialized/sdk',sdk['recipe_sha256'],armout/'materials.json')
                # Retain identities, static output, complete protocol and build
                # logs, not eighteen duplicate exported binary/source trees.
                shutil.rmtree(destination)
            require(set(identities)==set(index_api.ROLES),'cost-cell executable roles missing');write(armout/'identities.json',identities)
            records.append({**cell,'arm':arm,'whole_build_seconds':elapsed});write(armout/'total.json',records[-1])
    result={scenario:{arm:summary([r['whole_build_seconds'] for r in records if r['scenario']==scenario and r['arm']==arm]) for arm in ('sdk','chef')} for scenario in ('warm','source-comment','material-doc')}
    pairs=[]
    for cell in protocol['schedule']:
        values={r['arm']:r['whole_build_seconds'] for r in records if r['scenario']==cell['scenario'] and r['repetition']==cell['repetition']}
        pairs.append({**cell,'chef_minus_sdk_seconds':values['chef']-values['sdk']})
    write(output/'result.json',{'schema_version':1,'passed':True,'summaries':result,'paired_differences':pairs,'timed_builds':36,'all_materials_verified_against_each_actual_context':True,'exact_31_native_notices_match_sdk':True,'performance_claim':'descriptive paired shared-builder observations only; first restore and producer cost reported separately'})


if __name__=='__main__':main()
