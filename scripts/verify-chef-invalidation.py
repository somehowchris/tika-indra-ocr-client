#!/usr/bin/env python3
"""Execute fail-closed cooked-layer invalidations and real same-path static-link fixtures."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile
import time

ROOT=Path(__file__).resolve().parent.parent


def require(value,message):
    if not value:raise ValueError(message)


def write(path,value):
    with path.open('x') as f:json.dump(value,f,sort_keys=True,indent=2,allow_nan=False);f.write('\n')


def mutate(context,case):
    names={'source':'shared/trace-context/src/lib.rs','lock':'runtime/Cargo.lock','features':'runtime/crates/indra-engine/Cargo.toml','profile':'runtime/Cargo.toml'}
    path=context/names[case];before=path.read_bytes();text=before.decode()
    if case in ('source','lock'):text+='\n'+('//' if case=='source' else '#')+' Chef invalidation fixture\n'
    elif case=='features':
        require(text.count('[features]\n')==1,'feature fixture target differs');text=text.replace('[features]\n','[features]\nchef-invalidation-fixture = []\n',1)
    else:
        require(text.count('opt-level = 3')>=1,'release profile fixture differs');text=text.replace('opt-level = 3','opt-level = 2',1)
    path.write_text(text)
    return {'path':str(path.relative_to(context)),'before_sha256':hashlib.sha256(before).hexdigest(),'after_sha256':hashlib.sha256(path.read_bytes()).hexdigest()}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('index','inputs','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();index=json.loads(args.index.read_text());inputs=args.inputs.resolve();output=args.output.resolve()
    require(not output.exists() and not output.is_symlink(),'exclusive invalidation directory required');output.mkdir(mode=0o700)
    def run(name,command,expected_failure=False,diagnostics=(),seconds=1800):
        started=time.monotonic();status=None
        with (output/(name+'.stdout')).open('xb') as out,(output/(name+'.stderr')).open('xb') as err:
            try:status=subprocess.run([str(v) for v in command],cwd=ROOT,stdout=out,stderr=err,timeout=seconds).returncode
            except subprocess.TimeoutExpired:status=124
        write(output/(name+'.json'),{'command':[str(v) for v in command],'exit_code':status,'elapsed_seconds':time.monotonic()-started,'scope':'correctness/invalidation diagnostic, not performance comparison'})
        body=(output/(name+'.stderr')).read_text()+(output/(name+'.stdout')).read_text()
        if expected_failure:require(status not in (0,124) and any(d in body for d in diagnostics),'expected typed admission failure absent: '+name)
        else:require(status==0,'diagnostic command failed: '+name)
    sdk=json.loads((inputs/'sdk-materialized/materialization.json').read_text())
    common=['docker','buildx','build','--platform','linux/amd64','--progress=plain','--provenance=false','--sbom=false',
            '--build-context',f'sdk-input=oci-layout://{inputs}/sdk-oci@{index["inputs"]["sdk"]["digest"]}',
            '--build-arg','NATIVE_SDK_RECIPE_SHA256='+sdk['recipe_sha256'],'--build-arg','NATIVE_SDK_CONTENT_SHA256='+sdk['content_sha256'],
            '--build-arg','BUILD_CPUSET=0,1']
    # Each mutation starts from exact checked-out source bytes. These contexts
    # contain neither .git nor host target/cache directories.
    archive=output/'source.tar'
    with archive.open('xb') as f:subprocess.run(['git','-c','tar.umask=0022','archive','HEAD'],cwd=ROOT,stdout=f,check=True)
    changes=[]
    for case in ('source','lock','features','profile','sdk-content'):
        context=output/('context-'+case);context.mkdir()
        with tarfile.open(archive) as tar:tar.extractall(context,filter='data')
        if case!='sdk-content':changes.append({'case':case,**mutate(context,case)})
        dockerfile=output/(case+'.Dockerfile')
        run('render-'+case,['python3',ROOT/'scripts/render-chef-consumer.py','--source',context/'docker/native.Dockerfile','--mode','consumer','--output',dockerfile])
        command=[*common,'--file',dockerfile,'--target','chef-restored',
                 '--build-context',f'chef-input=oci-layout://{inputs}/chef-oci@{index["inputs"]["chef"]["digest"]}',
                 '--build-arg','CHEF_DEPENDENCY_KEY_SHA256='+index['inputs']['chef']['recipe_sha256'],
                 '--build-arg','CHEF_CONTENT_SHA256='+index['inputs']['chef']['content_sha256']]
        if case=='sdk-content':
            value='0'*64 if sdk['content_sha256']!='0'*64 else '1'*64
            command+=['--build-arg','NATIVE_SDK_CONTENT_SHA256='+value]
        command+=[context]
        run('reject-'+case,command,True,('source/lock/features/profile/SDK cooked dependency identity differs','source/lock/configuration changed during Chef prepare','SDK admission manifest identity differs','SDK content identity differs','SDK manifest hash differs'))
    write(output/'mutations.json',changes)
    producer=output/'fixture.Dockerfile'
    run('render-native-fixture',['python3',ROOT/'scripts/render-chef-consumer.py','--source',ROOT/'docker/native.Dockerfile','--mode','producer','--output',producer])
    with producer.open('a') as stream:stream.write((ROOT/'evaluation/build-cost/chef-native-fixture.Dockerfile.inc').read_text())
    receipts=[]
    for native,bonus in ((1,0),(2,0),(1,1)):
        name=f'native-{native}-source-{bonus}';destination=output/name
        run(name,[*common,'--file',producer,'--target','chef-native-fixture-result','--build-arg','NATIVE_FIXTURE_VALUE='+str(native),
                  '--build-arg','APP_FIXTURE_BONUS='+str(bonus),'--output','type=local,dest='+str(destination),ROOT])
        result=json.loads((destination/'result.json').read_text())
        require(result=={'native_value':native,'source_bonus':bonus,'observed':native+bonus,'source_mtime':1,'native_archive_path':'/native-fixture/libnative_answer.a'},'stale dummy or native link result')
        identities={line.split(maxsplit=1)[1]:line.split(maxsplit=1)[0] for line in (destination/'identities.sha256').read_text().splitlines()}
        receipts.append({'case':name,'result':result,'identities':identities})
    same_path=json.loads((output/'native-1-source-0/same-path-relink.json').read_text())
    require(same_path['passed'] is True and same_path['old_target_rejected'] is True and same_path['before_value']==1 and same_path['after_value']==2,'actual preserved-target same-path relink did not pass')
    require(receipts[0]['identities']['/native-fixture/libnative_answer.a']!=receipts[1]['identities']['/native-fixture/libnative_answer.a'],'same-path native archive did not change')
    require(receipts[0]['identities']['target/x86_64-unknown-linux-gnu/release/chef-native-fixture']!=receipts[2]['identities']['target/x86_64-unknown-linux-gnu/release/chef-native-fixture'],'old-mtime actual source reused dummy executable')
    write(output/'result.json',{'schema_version':1,'passed':True,'typed_rejections':5,'native_archive_values':[1,2],'old_source_mtime':1,'dummy_package_version':'0.0.1','fixtures':receipts,'performance_claim':False})


if __name__=='__main__':main()
