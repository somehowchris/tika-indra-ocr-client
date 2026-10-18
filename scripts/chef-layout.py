#!/usr/bin/env python3
"""Metadata-only cargo-chef workspace prototype. Never compiles dependencies or edits source."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


def require(ok,message):
    if not ok: raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):return json.loads(path.read_text())


def write(path,value):
    with path.open('x') as stream:
        json.dump(value,stream,indent=2,sort_keys=True,allow_nan=False)
        stream.write('\n')


def source_inputs(source):
    paths={source/'Cargo.toml',source/'Cargo.lock',source/'runtime/Cargo.toml',source/'runtime/Cargo.lock'}
    paths.update((source/'runtime/crates').glob('*/Cargo.toml'))
    paths.update(p for p in (source/'shared').rglob('*') if p.is_file() or p.is_symlink())
    result=[]
    for path in sorted(paths):
        require(path.is_file() and not path.is_symlink() and path==path.resolve(),'noncanonical Chef source input')
        result.append({'path':path.relative_to(source).as_posix(),'mode':path.stat().st_mode&0o7777,'sha256':sha(path)})
    return result


def cache_identity(root_recipe,runtime_recipe,external_inputs,sdk,chef,helper_sha256):
    for key in ('recipe_sha256','content_sha256'):
        require(isinstance(sdk.get(key),str) and re.fullmatch(r'[0-9a-f]{64}',sdk[key]) is not None,'SDK exact identity missing')
    require(re.fullmatch(r'sha256:[0-9a-f]{64}',sdk.get('platform_manifest_digest','')) is not None,'SDK immutable manifest missing')
    value={'schema_version':1,'root_recipe_sha256':root_recipe,'runtime_recipe_sha256':runtime_recipe,
           'external_workspace_inputs':external_inputs,'sdk_recipe_sha256':sdk['recipe_sha256'],'sdk_content_sha256':sdk['content_sha256'],
           'chef_binary_sha256':chef['binary_sha256'],'layout_helper_sha256':helper_sha256,
           'compiler_contract':{'target':'x86_64-unknown-linux-gnu','profile':'release','rustflags':'-Ctarget-feature=+crt-static','cargo_jobs':2,
                                'default_features':'unchanged','packages':['indra-engine','indra-server'],'cook_targets':['tests','examples'],
                                'final_commands':'all four original build/test commands preserved after restoring actual source; seven-role bytes/static/lifecycle gate mandatory'}}
    digest=hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    return {**value,'compiled_dependency_key':digest,'claim':'prospective identity only; no cooked artifacts, transported target state, fresh runner or speedup demonstrated'}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--source-commit',required=True)
    parser.add_argument('--chef',type=Path,required=True)
    parser.add_argument('--sdk-receipt',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    require(args.source.is_absolute() and args.source==args.source.resolve(),'canonical absolute source required')
    require(re.fullmatch(r'[0-9a-f]{40}',args.source_commit) is not None,'full source commit required')
    require(not args.output.exists() and not args.output.is_symlink(),'exclusive output directory required')
    require(not args.output.resolve().is_relative_to(args.source),'layout must remain outside immutable source')
    lock=read(Path(__file__).with_name('cargo-chef.lock.json'))
    require(args.chef.is_file() and not args.chef.is_symlink() and sha(args.chef)==lock['binary_sha256'],'locked chef executable identity differs')
    require(subprocess.check_output([args.chef,'--version'],text=True).strip()=='cargo-chef '+lock['version'],'locked chef version differs')
    before=source_inputs(args.source)
    args.output.mkdir()
    commands=[]
    environment={**os.environ,'CARGO_NET_OFFLINE':'true','CARGO':shutil.which('cargo') or ''}
    require(bool(environment['CARGO']),'Cargo executable missing')
    def run(name,command,cwd):
        with (args.output/(name+'.stdout')).open('xb') as stdout,(args.output/(name+'.stderr')).open('xb') as stderr:
            done=subprocess.run(command,cwd=cwd,env=environment,stdout=stdout,stderr=stderr)
        commands.append({'phase':name,'command':[str(x) for x in command],'cwd':str(cwd),'exit_code':done.returncode,'scope':'prepare/no-build/metadata only; dependency compilation is never requested'})
        write(args.output/(name+'.command.json'),commands[-1])
        require(done.returncode==0,'Chef layout failed; outputs retained: '+name)
    for name,cwd in (('root',args.source),('runtime',args.source/'runtime')):
        run(name+'-prepare',[args.chef,'chef','prepare','--recipe-path',args.output/(name+'-recipe.json')],cwd)
    layout=args.output/'layout';layout.mkdir()
    run('root-skeleton',[args.chef,'chef','cook','--no-build','--recipe-path',args.output/'root-recipe.json'],layout)
    for name in ('Cargo.toml','Cargo.lock'):shutil.copyfile(args.source/name,layout/name)
    require((layout/'shared').is_dir() and not (layout/'shared').is_symlink(),'root Chef skeleton omitted shared workspace')
    shutil.rmtree(layout/'shared') # Only our newly generated private dummy directory.
    shutil.copytree(args.source/'shared',layout/'shared')
    (layout/'runtime').mkdir()
    run('runtime-skeleton',[args.chef,'chef','cook','--no-build','--recipe-path',args.output/'runtime-recipe.json'],layout/'runtime')
    run('runtime-metadata',['cargo','metadata','--no-deps','--locked','--offline','--format-version','1','--manifest-path','runtime/Cargo.toml'],layout)
    after=source_inputs(args.source)
    require(before==after,'immutable source inputs changed during prepare')
    # Runtime recipe captures its workspace manifests/lock; the owning root and
    # actual external shared source are additional explicit dependency inputs.
    external=[entry for entry in before if not entry['path'].startswith('runtime/')]
    identity=cache_identity(sha(args.output/'root-recipe.json'),sha(args.output/'runtime-recipe.json'),external,read(args.sdk_receipt),lock,sha(Path(__file__)))
    write(args.output/'cache-identity.json',identity)
    write(args.output/'admission.json',{'schema_version':1,'source_commit':args.source_commit,'source_inputs_unchanged':True,'commands':commands,
          'external_root_and_shared_source_preserved':True,'native_or_rust_compilation_requested':False,'sdk_receipt_sha256':sha(args.sdk_receipt),
          'scope':'actual Chef source/layout compatibility only; all compiled-cache/equivalence/performance gates remain pending'})
    print(json.dumps({'layout_admitted':True,'compiled_dependency_key':identity['compiled_dependency_key'],'compilation_requested':False}))


if __name__=='__main__':main()
