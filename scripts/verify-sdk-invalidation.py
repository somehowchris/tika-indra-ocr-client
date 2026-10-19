#!/usr/bin/env python3
"""Reject deliberate copies of the current real SDK; never alter its signed inputs."""
from __future__ import annotations
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT=Path(__file__).resolve().parent.parent


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value


sdk_api=module('sdk_invalidation_reader',ROOT/'scripts/native-sdk.py')
audit=module('sdk_invalidation_audit',ROOT/'evaluation/build-cost/audit.py')


def require(value,message):
    if not value:raise ValueError(message)


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,sort_keys=True,indent=2,allow_nan=False);stream.write('\n')


def require_rejection(status,body,diagnostics):
    require(status==1 and any(text in body for text in diagnostics),'expected typed SDK admission failure absent')


def mutate_blob(layout,digest,case):
    """Mutate one actual layer only after verifying this independent copy."""
    require(case in ('missing','truncated','corrupt'),'unknown SDK blob mutation')
    receipt=sdk_api.verify_oci(layout,digest)
    manifest=sdk_api.read(layout/'blobs/sha256'/receipt['platform_manifest_digest'].split(':')[1])
    descriptor=manifest['layers'][0];path=layout/'blobs/sha256'/descriptor['digest'].split(':')[1]
    before={'sha256':sdk_api.sha(path),'bytes':path.stat().st_size}
    require(before['bytes']>0,'nonempty actual SDK layer required')
    if case=='missing':path.unlink()
    else:
        with path.open('r+b') as stream:
            if case=='truncated':stream.truncate(before['bytes']-1)
            else:
                first=stream.read(1);stream.seek(0);stream.write(bytes([first[0]^1]))
    return {'case':case,'path':str(path.relative_to(layout)),'original_descriptor':descriptor,
            'mutation_rule':{'missing':'unlink selected layer blob','truncated':'truncate selected layer blob to original bytes minus one','corrupt':'XOR first byte of selected layer blob with 1; preserve size'}[case],'before':before,
            'after':{'sha256':sdk_api.sha(path),'bytes':path.stat().st_size} if path.exists() else None}


def remove_owned_copy(copied,output,case):
    require(case in ('missing','truncated','corrupt') and copied==output/('oci-'+case)
            and copied==copied.resolve() and copied.is_dir() and not copied.is_symlink(),'refused SDK copy cleanup outside exact owned case')
    shutil.rmtree(copied)
    require(not copied.exists() and not copied.is_symlink(),'owned SDK case copy remains after cleanup')


def diagnostic_recipe(original,case):
    marker='RUN python3 /sdk-input/scripts/native-sdk.py verify '
    require(original.count(marker)==1,'unique real SDK consumer admission required')
    additions={
        'compiler':'''RUN mkdir /sdk-negative-compiler && cp -L "$(command -v gcc)" /sdk-negative-compiler/gcc \\
    && printf '\\000' >> /sdk-negative-compiler/gcc \\
    && sha256sum "$(command -v gcc)" /sdk-negative-compiler/gcc \\
    && /sdk-negative-compiler/gcc --version
ENV PATH=/sdk-negative-compiler:${PATH}
''',
        'source':"RUN printf '\\n# Deliberately changed SDK input source\\n' >> /sdk-input/scripts/build-native-runtime.sh\n",
        'closure':"RUN printf '\\n# Deliberately changed restored SDK file\\n' >> /native/cargo-env.sh\n",
    }
    if case=='platform':
        before='--platform "$TARGETPLATFORM"';require(original.count(before)==1,'unique consumer platform argument required')
        return original.replace(before,'--platform "linux/arm64"',1)
    require(case in additions,'unknown diagnostic recipe mutation')
    return original.replace(marker,additions[case]+marker,1)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('index','inputs','output'):parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();inputs=args.inputs.resolve();output=args.output.resolve();index=sdk_api.read(args.index)
    require(not output.exists() and not output.is_symlink(),'exclusive SDK invalidation directory required');output.mkdir(mode=0o700)
    context=inputs/'source-context'
    require(context.is_dir() and not context.is_symlink(),'actual fresh proof source context required')
    sdk=sdk_api.read(inputs/'sdk-materialized/materialization.json');expected=index['inputs']['sdk']
    require(sdk['recipe_sha256']==expected['recipe_sha256'] and sdk['content_sha256']==expected['content_sha256'],'current SDK materialization/index identity differs')
    layout=inputs/'sdk-oci';before=audit.inventory(layout)
    sdk_api.verify_oci(layout,expected['digest'])
    write(output/'canonical-before.json',before)
    def reject(name,command,diagnostics,seconds=1800):
        started=time.monotonic()
        with (output/(name+'.stdout')).open('xb') as stdout,(output/(name+'.stderr')).open('xb') as stderr:
            try:status=subprocess.run([str(v) for v in command],cwd=ROOT,stdout=stdout,stderr=stderr,timeout=seconds).returncode
            except subprocess.TimeoutExpired:status=124
        body=(output/(name+'.stdout')).read_text()+(output/(name+'.stderr')).read_text()
        write(output/(name+'.json'),{'command':[str(v) for v in command],'exit_code':status,'elapsed_seconds':time.monotonic()-started,
             'required_diagnostics':list(diagnostics),'scope':'deliberately modified real SDK rejection, not a signature or performance result'})
        require_rejection(status,body,diagnostics)
    try:
        for case in ('missing','truncated','corrupt'):
            copied=output/('oci-'+case)
            cleanup={'path':str(copied),'expected_rejection_passed':False,'removed_after_expected_rejection':False}
            try:
                shutil.copytree(layout,copied)
                mutation=mutate_blob(copied,expected['digest'],case);write(output/(case+'-mutation.json'),mutation)
                diagnostics=('not a canonical regular file: '+str(copied/mutation['path']),) if case=='missing' else ('missing/corrupt/truncated OCI blob: '+expected_layer_digest(mutation),)
                reject('reject-'+case,['python3',ROOT/'scripts/native-sdk.py','materialize-oci','--layout',copied,'--digest',expected['digest'],'--inputs',context,'--output',output/('rejected-'+case)],diagnostics)
                cleanup['expected_rejection_passed']=True
                write(output/(case+'-mutated-inventory.json'),audit.inventory(copied))
                remove_owned_copy(copied,output,case)
                cleanup['removed_after_expected_rejection']=True
            finally:
                cleanup['copy_retained_for_failure']=copied.exists() or copied.is_symlink()
                write(output/(case+'-cleanup.json'),cleanup)
        original=(inputs/'sdk.Dockerfile').read_text()
        # Target native-build stops after the actual SDK verifier: no C++ source
        # build and no application Cargo command is part of these negatives.
        for case in ('recipe','content','compiler','source','closure','platform','configuration'):
            dockerfile=output/(case+'.Dockerfile')
            dockerfile.write_text(original if case in ('recipe','content','configuration') else diagnostic_recipe(original,case))
            recipe=sdk['recipe_sha256'];content=sdk['content_sha256'];cpuset='0,1'
            if case=='recipe':recipe='0'*64 if recipe!='0'*64 else '1'*64
            if case=='content':content='0'*64 if content!='0'*64 else '1'*64
            if case=='configuration':cpuset='1'
            diagnostics=('SDK admission manifest identity differs',) if case in ('recipe','content') else (
                'restored SDK closure path/type/mode/size/hash differs',) if case=='closure' else (
                'unsupported SDK platform',) if case=='platform' else ('SDK recipe/toolchain/native configuration differs',)
            reject('reject-'+case,['docker','buildx','build','--platform','linux/amd64','--progress=plain','--file',dockerfile,'--target','native-build',
                '--build-context',f'sdk-input=oci-layout://{layout}@{expected["digest"]}',
                '--build-arg','NATIVE_SDK_RECIPE_SHA256='+recipe,'--build-arg','NATIVE_SDK_CONTENT_SHA256='+content,
                '--build-arg','BUILD_CPUSET='+cpuset,'--provenance=false','--sbom=false',context],diagnostics)
    finally:
        after=audit.inventory(layout);write(output/'canonical-after.json',after)
        require(before==after,'canonical signed OCI input changed during SDK negatives')
    write(output/'result.json',{'schema_version':1,'passed':True,'source_sha':index['source_sha'],'sdk_digest':expected['digest'],
        'recipe_sha256':sdk['recipe_sha256'],'content_sha256':sdk['content_sha256'],'real_copied_blob_rejections':3,'real_consumer_rejections':7,
        'canonical_sdk_unchanged':True,'expected_rejection_copies_removed':3,'mutations_reproducible_from_canonical_and_retained_rules':True,'native_source_compilation':False,'application_compilation':False,'performance_claim':False})


def expected_layer_digest(mutation):return 'sha256:'+Path(mutation['path']).name


if __name__=='__main__':main()
