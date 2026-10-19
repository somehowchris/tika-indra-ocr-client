#!/usr/bin/env python3
"""Diagnostic-only old-target/new-archive proof at a fixed native pathname."""
from __future__ import annotations
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess


def write(path,value):
    with path.open('x') as f:json.dump(value,f,indent=2,sort_keys=True);f.write('\n')


def main():
    spec=importlib.util.spec_from_file_location('chef_artifact','/build-tools/chef-artifact.py');chef=importlib.util.module_from_spec(spec);spec.loader.exec_module(chef)
    output=Path('/fixture-evidence');archive=Path('/native-fixture/libnative_answer.a');binary=Path('target/x86_64-unknown-linux-gnu/release/chef-native-fixture');roots={'target':Path('target').resolve()}
    def native_content():return {'schema_version':1,'entries':[{'path':str(archive),'bytes':archive.stat().st_size,'sha256':chef.sha(archive)}]}
    def identity(content):
        contract={'native_recipe':'diagnostic tiny C archive, gcc -c -O2; ar rcsD; fixed absolute pathname','sdk_content':content}
        return {'schema_version':1,'dependency_key_sha256':hashlib.sha256(chef.canonical(contract)).hexdigest(),'inputs':contract}
    def run(name,command):
        with (output/(name+'.stdout')).open('xb') as stdout,(output/(name+'.stderr')).open('xb') as stderr:
            subprocess.run(command,stdout=stdout,stderr=stderr,check=True,timeout=120)
    before=native_content();old=identity(before)
    write(output/'old-native-content.json',before);write(output/'old-key.json',old)
    write(output/'old-target-content.json',{'schema_version':1,'dependency_key_sha256':old['dependency_key_sha256'],'entries':chef.inventory(roots)})
    if subprocess.check_output([str(binary.resolve())],text=True).strip()!='1':raise ValueError('old target must actually return one')
    # Preserve target/fingerprints and all original source paths/old mtimes.
    Path('/native-fixture/answer.c').write_text('unsigned native_answer(void) { return 2; }\n')
    run('new-native-compile',['gcc','-c','-O2','/native-fixture/answer.c','-o','/native-fixture/answer.o'])
    run('new-native-archive',['ar','rcsD',str(archive),'/native-fixture/answer.o'])
    after=native_content();current=identity(after)
    if before==after:raise ValueError('archive bytes did not change at the same pathname')
    write(output/'new-native-content.json',after);write(output/'new-key.json',current)
    try:
        chef.verify(output/'old-target-content.json',output/'old-key.json',old['dependency_key_sha256'],chef.sha(output/'old-target-content.json'),output/'new-key.json',roots)
    except ValueError as error:
        if str(error)!='source/lock/features/profile/SDK cooked dependency identity differs':raise
        write(output/'old-target-rejection.json',{'passed':True,'error':str(error)})
    else:raise ValueError('old target admitted against a new actual native archive identity')
    command=['cargo','build','--release','--locked','--target','x86_64-unknown-linux-gnu','--jobs','2']
    run('warm-native-relink',command)
    if subprocess.check_output([str(binary.resolve())],text=True).strip()!='2':raise ValueError('stale warm target after native archive changed')
    warm=chef.sha(binary)
    run('clean-native-state',['cargo','clean','--release','--target','x86_64-unknown-linux-gnu'])
    run('clean-native-rebuild',command)
    if subprocess.check_output([str(binary.resolve())],text=True).strip()!='2' or chef.sha(binary)!=warm:raise ValueError('warm native relink differs from clean value-two build')
    write(output/'same-path-relink.json',{'schema_version':1,'passed':True,'before_value':1,'after_value':2,'archive_path':str(archive),'old_target_rejected':True,'warm_and_clean_binary_sha256':warm,'native_content_before_sha256':chef.sha(output/'old-native-content.json'),'native_content_after_sha256':chef.sha(output/'new-native-content.json'),'scope':'actual tiny diagnostic native content and preserved old compiled target; not a production ONNX SDK equivalence or performance claim'})


if __name__=='__main__':main()
