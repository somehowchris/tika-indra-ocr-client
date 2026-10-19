#!/usr/bin/env python3
"""Build-only identity/content admission for the opt-in cooked dependency artifact."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ROOTS={'target':Path('/app/runtime/target'),'registry':Path('/usr/local/cargo/registry'),'git':Path('/usr/local/cargo/git')}


def require(value,message):
    if not value:raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size<=67108864,'invalid bounded Chef manifest')
    return json.loads(path.read_text())


def canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False).encode()


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False);stream.write('\n')


def source_inputs(source):
    paths={source/'Cargo.toml',source/'Cargo.lock',source/'runtime/Cargo.toml',source/'runtime/Cargo.lock'}
    paths.update((source/'runtime/crates').glob('*/Cargo.toml'))
    paths.update(p for p in (source/'shared').rglob('*') if p.is_file() or p.is_symlink())
    result=[]
    # Absence is part of the key: adding a previously absent config/toolchain
    # invalidates the artifact instead of attempting COPY from a missing path.
    for name in ('.cargo','rust-toolchain.toml','runtime/.cargo','runtime/rust-toolchain.toml'):
        path=source/name
        require(not path.is_symlink(),'symlink optional Cargo input')
        if not path.exists():
            result.append({'path':name,'kind':'absent'})
        elif path.is_file():paths.add(path)
        else:
            require(path.is_dir(),'unsupported optional Cargo input')
            result.append({'path':name,'kind':'directory','mode':path.stat().st_mode&0o7777})
            paths.update(p for p in path.rglob('*') if p.is_file() or p.is_symlink())
    for path in sorted(paths):
        require(path.is_file() and not path.is_symlink(),'missing/symlink Chef input: '+str(path))
        result.append({'path':str(path.relative_to(source)),'mode':path.stat().st_mode&0o7777,'sha256':sha(path)})
    return sorted(result,key=lambda entry:entry['path'])


def application_environment(source):
    text=(source/'docker/native.Dockerfile').read_text()
    stage=text.split('FROM native-build AS application-build\n')
    require(len(stage)==2,'shared reproducible application stage missing or ambiguous')
    body=stage[1].split('\nFROM ',1)[0]
    values=re.findall(r'^ENV SOURCE_DATE_EPOCH=(0|[1-9][0-9]{0,9})$',body,re.MULTILINE)
    require(len(values)==1,'pinned application SOURCE_DATE_EPOCH missing or ambiguous')
    return {'SOURCE_DATE_EPOCH':values[0]}


def key(source,root_recipe,runtime_recipe,sdk_recipe,sdk_content,helper_directory,captured_inputs=None):
    actual_inputs=source_inputs(source)
    environment=application_environment(source)
    require(all(os.environ.get(name)==value for name,value in environment.items()),
            'actual Chef planner environment differs from pinned application environment')
    if captured_inputs is not None:require(actual_inputs==captured_inputs,'source/lock/configuration changed during Chef prepare')
    tools={name:sha(helper_directory/name) for name in ('chef-artifact.py','render-chef-consumer.py','cargo-chef.lock.json')}
    contract={'schema_version':1,'root_recipe_sha256':sha(root_recipe),'runtime_recipe_sha256':sha(runtime_recipe),
              'workspace_inputs':actual_inputs,'sdk_recipe_sha256':sha(sdk_recipe),'sdk_content_sha256':sha(sdk_content),
              'build_tools':tools,'contract':{'container_workspaces':['/app','/app/runtime'],'shared_path':'/app/shared',
              'target':'x86_64-unknown-linux-gnu','profile':'release','packages':['indra-engine','indra-server'],
              'targets':['tests','examples'],'default_features':True,'jobs':2,'rustflags':'-Ctarget-feature=+crt-static',
              'application_environment':environment,
              'compiler':'1.99.1-x86_64-unknown-linux-gnu; exact compiler bytes additionally bound by SDK recipe',
              'source_commands':'all four original Cargo invocations execute after restoring actual source'}}
    return {'schema_version':1,'dependency_key_sha256':hashlib.sha256(canonical(contract)).hexdigest(),'inputs':contract}


def inventory(roots):
    result=[];total=0
    for name,root in roots.items():
        require(root.is_dir() and not root.is_symlink(),'missing compiled dependency root: '+name)
        for path in [root,*sorted(root.rglob('*'))]:
            require(not path.is_symlink() and (path.is_file() or path.is_dir()),'unsupported compiled cache entry')
            mode=path.stat().st_mode&0o7777;require(mode&0o6000==0,'setuid/setgid cache entry')
            size=path.stat().st_size if path.is_file() else 0;total+=size
            require(total<=8589934592 and len(result)<100000,'compiled cache exceeds8GiB/100000 entries')
            result.append({'path':str(Path(name)/path.relative_to(root)),'type':'file' if path.is_file() else 'directory',
                           'mode':mode,'bytes':size,'sha256':sha(path) if path.is_file() else None})
    return sorted(result,key=lambda entry:entry['path'])


def verify(manifest,keyfile,expected_key,expected_content,current_key,roots):
    require(all(re.fullmatch('[0-9a-f]{64}',value) for value in (expected_key,expected_content)),'expected Chef identities missing')
    require(sha(manifest)==expected_content,'compiled cache content manifest differs')
    locked=read(keyfile);current=read(current_key)
    require(locked['dependency_key_sha256']==current['dependency_key_sha256']==expected_key,'source/lock/features/profile/SDK cooked dependency identity differs')
    require(locked==current,'cooked dependency recipe inputs differ')
    content=read(manifest)
    require(content['schema_version']==1 and content['dependency_key_sha256']==expected_key,'compiled content/key binding differs')
    require(content['entries']==inventory(roots),'compiled target/registry/git bytes or modes differ')


def clean_workspace(output):
    # Upstream0.1.78 removes libraries/build scripts, not every dummy binary,
    # test or example. Clean every local workspace package explicitly while the
    # normalized recipe is active; keep registry and real external shared deps.
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],text=True))
    members=set(metadata['workspace_members'])
    names=sorted(p['name'] for p in metadata['packages'] if p['id'] in members)
    require(names and len(names)==len(members)==len(set(names)),'workspace cleanup package identity differs')
    command=['cargo','clean','--release','--target','x86_64-unknown-linux-gnu']
    for name in names:command+=['--package',name]
    subprocess.run(command,check=True)
    write(output,{'schema_version':1,'packages':names,'command':command,
                  'scope':'all local normalized workspace artifacts removed; registry and real external shared dependency artifacts retained'})


def main():
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest='action',required=True)
    k=sub.add_parser('key')
    for name in ('source','root-recipe','runtime-recipe','sdk-recipe','sdk-content','output'):k.add_argument('--'+name,type=Path,required=True)
    k.add_argument('--source-inputs',type=Path,required=True)
    snap=sub.add_parser('snapshot');snap.add_argument('--source',type=Path,required=True);snap.add_argument('--output',type=Path,required=True)
    clean=sub.add_parser('clean-workspace');clean.add_argument('--output',type=Path,required=True)
    m=sub.add_parser('manifest');m.add_argument('--key',type=Path,required=True);m.add_argument('--output',type=Path,required=True)
    v=sub.add_parser('verify')
    for name in ('manifest','key','current-key'):v.add_argument('--'+name,type=Path,required=True)
    for name in ('expected-key','expected-content'):v.add_argument('--'+name,required=True)
    args=parser.parse_args()
    if args.action=='key':write(args.output,key(args.source,args.root_recipe,args.runtime_recipe,args.sdk_recipe,args.sdk_content,Path(__file__).parent,read(args.source_inputs)))
    elif args.action=='snapshot':write(args.output,source_inputs(args.source))
    elif args.action=='clean-workspace':clean_workspace(args.output)
    elif args.action=='manifest':write(args.output,{'schema_version':1,'dependency_key_sha256':read(args.key)['dependency_key_sha256'],'entries':inventory(ROOTS)})
    else:verify(args.manifest,args.key,args.expected_key,args.expected_content,args.current_key,ROOTS)


if __name__=='__main__':main()
