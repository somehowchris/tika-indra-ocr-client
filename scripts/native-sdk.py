#!/usr/bin/env python3
"""Build-only local native SDK closure packaging/admission; no implicit source fallback."""
from __future__ import annotations
import argparse
import hashlib
import json
import gzip
import tarfile
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess

APP_MARKER = '# Application compilation is cached independently'
INPUTS = ('scripts/native-build-tools.lock', 'scripts/build-native-runtime.sh', 'scripts/record-native-runtime.sh',
          'scripts/collect-runtime-materials.sh', 'scripts/native-sdk.py')
SHA = re.compile(r'^[0-9a-f]{64}$')
OCI_INDEX = 'application/vnd.oci.image.index.v1+json'
OCI_MANIFEST = 'application/vnd.oci.image.manifest.v1+json'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    require(path.stat().st_size <= 16777216, 'SDK JSON exceeds 16MiB')
    return json.loads(path.read_text(), parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))


def write(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')


def regular(path):
    require(path.is_file() and not path.is_symlink() and path == path.resolve(), 'not a canonical regular file: '+str(path))


def prefix(source):
    text = source.read_text()
    require(text.count(APP_MARKER) == 1, 'native-stage delimiter changed')
    return text.split(APP_MARKER)[0]


def notice_paths(collector):
    text = collector.read_text()
    marker = "done <<'NATIVE_NOTICES'\n"
    require(text.count(marker) == 1, 'required notice source delimiter changed')
    body = text.split(marker)[1].split('\nNATIVE_NOTICES\n')[0]
    paths = []
    for line in body.splitlines():
        component, relative = line.split('|')
        path = PurePosixPath(relative)
        require(component and not path.is_absolute() and '..' not in path.parts, 'invalid notice source path')
        paths.append(path.as_posix())
    require(len(paths) == 31 and len(set(paths)) == 31, 'required notice closure is not exactly 31 unique paths')
    return paths


def entries(root, names=None):
    paths = sorted(root.rglob('*')) if names is None else sorted(p for name in names for p in [root/name, *(root/name).rglob('*')])
    result = []
    for path in paths:
        require(not path.is_symlink() and (path.is_file() or path.is_dir()), 'unsupported closure entry: '+str(path))
        result.append({'path':path.relative_to(root).as_posix(), 'type':'file' if path.is_file() else 'directory',
                       'mode':path.stat().st_mode & 0o7777, 'bytes':path.stat().st_size if path.is_file() else None,
                       'sha256':sha(path) if path.is_file() else None})
    require(bool(result), 'empty native SDK closure')
    return result


def copy_file(source, destination):
    regular(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    require(not destination.exists() and not destination.is_symlink(), 'duplicate SDK destination: '+str(destination))
    shutil.copy2(source, destination)


def source_recipe(inputs):
    source = inputs/'docker/native.Dockerfile'
    regular(source)
    files = {}
    for name in INPUTS:
        regular(inputs/name)
        files[name] = sha(inputs/name)
    return {'native_docker_prefix_sha256': hashlib.sha256(prefix(source).encode()).hexdigest(), 'source_files':files}


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def toolchain():
    names = ('rustc', 'cargo', 'gcc', 'g++', 'ld', 'ar', 'clang-18', 'cmake', 'ninja', 'nasm')
    tools = {}
    for name in names:
        found = shutil.which(name)
        require(found is not None, 'missing SDK compiler tool: '+name)
        actual = Path(found).resolve()
        if name in ('rustc','cargo'):
            actual = Path(command('rustc','--print','sysroot'))/'bin'/name
        regular(actual)
        tools[name] = {'path':str(actual), 'sha256':sha(actual), 'version':command(name, '-Vv' if name=='rustc' else '-v' if name=='nasm' else '--version')}
    return {'tools':tools, 'packages':command('dpkg-query','-W','-f=${binary:Package}\t${Version}\t${Architecture}\n').splitlines(),
            'glibc':command('getconf','GNU_LIBC_VERSION')}


def recipe(inputs, native, platform, debian_snapshot, build_cpuset):
    require(platform == 'linux/amd64', 'unsupported SDK platform')
    require(re.fullmatch(r'[0-9]{8}T[0-9]{6}Z',debian_snapshot) is not None, 'invalid Debian snapshot')
    require(re.fullmatch(r'[0-9]+(?:[-,][0-9]+)*',build_cpuset) is not None, 'invalid producer compiler affinity')
    configs = ('native-build-tools.lock','onnxruntime-CMakeCache.txt','opencv-CMakeCache.txt','opencv-cvconfig.h','static-archives.sha256')
    configuration = {}
    for name in configs:
        regular(native/'evidence'/name)
        configuration[name] = sha(native/'evidence'/name)
    return {'schema_version':1, 'platform':platform, 'native_build_arguments':{'DEBIAN_SNAPSHOT':debian_snapshot,'BUILD_CPUSET':build_cpuset}, 'source':source_recipe(inputs), 'toolchain':toolchain(),
            'native_configuration':configuration, 'notice_paths':notice_paths(inputs/'scripts/collect-runtime-materials.sh'),
            'identity_scope':'recipe/compiler/native configuration; actual archive/header/material file bytes separately bound by content.json; application source commit is provenance, not an SDK cache key'}


def check_native_manifests(native):
    require(native == Path('/native'), 'native build paths must remain /native')
    for name in ('SHA256SUMS','static-archives.sha256'):
        regular(native/'evidence'/name)
        subprocess.run(['sha256sum','--check','--status',str(native/'evidence'/name)], check=True)


def package(args):
    require(re.fullmatch(r'[0-9a-f]{40}', args.source_commit) is not None, 'full immutable producer source commit required')
    check_native_manifests(args.native)
    args.sdk.mkdir()
    root = args.sdk/'rootfs'
    root.mkdir()
    selected = set()
    for base in (args.native/'build/onnxruntime/Release', args.native/'opencv/lib'):
        archives = list(base.rglob('*.a'))
        require(bool(archives), 'missing actual native archives: '+str(base))
        selected.update(archives)
    for base in (args.native/'opencv/include', args.native/'evidence', Path('/glibc-source')):
        require(base.is_dir() and not base.is_symlink(), 'missing SDK closure directory: '+str(base))
        selected.update(p for p in base.rglob('*') if not p.is_dir())
    selected.add(args.native/'cargo-env.sh')
    for name in notice_paths(args.inputs/'scripts/collect-runtime-materials.sh'):
        selected.add(args.native/name)
    for path in sorted(selected):
        copy_file(path, root/path.relative_to('/'))
    # Complete closure verification occurs before the artifact is exportable.
    actual_recipe = recipe(args.inputs,args.native,args.platform,args.debian_snapshot,args.build_cpuset)
    write(args.sdk/'recipe.json',actual_recipe)
    write(args.sdk/'content.json', {'schema_version':1, 'roots':['glibc-source','native'], 'entries':entries(root)})
    write(args.sdk/'producer.json', {'schema_version':1, 'source_commit':args.source_commit, 'recipe_sha256':sha(args.sdk/'recipe.json'),
                                     'content_sha256':sha(args.sdk/'content.json'), 'claim':'local diagnostic artifact; no signing or fresh-builder/executable-equivalence claim'})
    print(json.dumps(read(args.sdk/'producer.json')))


def verify(args):
    require(SHA.fullmatch(args.recipe_sha256) is not None and SHA.fullmatch(args.content_sha256) is not None, 'exact expected recipe/content SHA256 required')
    require(sha(args.sdk/'recipe.json') == args.recipe_sha256 and sha(args.sdk/'content.json') == args.content_sha256, 'SDK admission manifest identity differs')
    locked = read(args.sdk/'recipe.json')
    actual = recipe(args.inputs,args.native,args.platform,args.debian_snapshot,args.build_cpuset)
    require(locked == actual, 'SDK recipe/toolchain/native configuration differs; source rebuild is an explicit separate command')
    content = read(args.sdk/'content.json')
    require(content.get('schema_version') == 1 and content.get('roots') == ['glibc-source','native'], 'unsupported SDK closure roots')
    require(entries(args.rootfs,content['roots']) == content['entries'], 'restored SDK closure path/type/mode/size/hash differs')
    check_native_manifests(args.native)
    for name in notice_paths(args.inputs/'scripts/collect-runtime-materials.sh'):
        regular(args.native/name)
    print(json.dumps({'recipe_sha256':args.recipe_sha256,'content_sha256':args.content_sha256,'restored_entries':len(content['entries']),
                      'scope':'restored closure/compiler admission only; application executable and lifecycle gates remain required'}))


def render(args):
    source = args.source.read_text()
    if args.action == 'render-producer':
        generated = prefix(args.source)+'''FROM native-build AS native-sdk-package
ARG SOURCE_COMMIT
ARG TARGETPLATFORM
ARG DEBIAN_SNAPSHOT
ARG BUILD_CPUSET
COPY docker/native.Dockerfile /sdk-input/docker/native.Dockerfile
COPY scripts/native-sdk.py scripts/native-build-tools.lock scripts/build-native-runtime.sh scripts/record-native-runtime.sh scripts/collect-runtime-materials.sh /sdk-input/scripts/
RUN python3 /sdk-input/scripts/native-sdk.py package --inputs /sdk-input --sdk /sdk --platform "$TARGETPLATFORM" --source-commit "$SOURCE_COMMIT" --debian-snapshot "$DEBIAN_SNAPSHOT" --build-cpuset "$BUILD_CPUSET"
FROM scratch AS native-sdk
COPY --from=native-sdk-package /sdk /sdk
'''
    else:
        start = source.index('FROM native-tools AS native-build\n')
        end = source.index('\n# This export target',start)
        require(source[start:end].count('build-native-runtime.sh /native')==1, 'source native compile stage changed')
        generated = source[:start]+'''# Mandatory immutable named context: sdk-input=oci-layout:///...@sha256:...
# An absent/incompatible SDK fails; original source Dockerfile remains separate.
FROM sdk-input AS native-sdk-input
FROM native-tools AS native-build
ARG TARGETPLATFORM
ARG BUILD_CPUSET=0,1
ARG NATIVE_SDK_RECIPE_SHA256
ARG NATIVE_SDK_CONTENT_SHA256
COPY docker/native.Dockerfile /sdk-input/docker/native.Dockerfile
COPY scripts/native-sdk.py scripts/native-build-tools.lock scripts/build-native-runtime.sh scripts/record-native-runtime.sh scripts/collect-runtime-materials.sh /sdk-input/scripts/
COPY --from=native-sdk-input /sdk/rootfs/native /native
COPY --from=native-sdk-input /sdk/rootfs/glibc-source /glibc-source
COPY --from=native-sdk-input /sdk/recipe.json /sdk/content.json /sdk-admission/
RUN python3 /sdk-input/scripts/native-sdk.py verify --inputs /sdk-input --sdk /sdk-admission --rootfs / --platform "$TARGETPLATFORM" --recipe-sha256 "$NATIVE_SDK_RECIPE_SHA256" --content-sha256 "$NATIVE_SDK_CONTENT_SHA256" --debian-snapshot "$DEBIAN_SNAPSHOT" --build-cpuset "$BUILD_CPUSET"
''' +source[end:]
    with args.output.open('x') as stream:
        stream.write(generated)


def verify_oci(layout, expected):
    require(re.fullmatch(r'sha256:[0-9a-f]{64}',expected) is not None, 'immutable OCI digest required')
    require(read(layout/'oci-layout') == {'imageLayoutVersion':'1.0.0'}, 'unsupported OCI layout')
    index = read(layout/'index.json')
    require(index.get('schemaVersion') == 2, 'invalid OCI index')
    seen = {}
    selected = []
    identities = set()
    def blob(desc):
        require(isinstance(desc,dict) and isinstance(desc.get('size'),int) and not isinstance(desc['size'],bool) and desc['size']>=0, 'invalid OCI blob descriptor size')
        digest = desc.get('digest')
        require(isinstance(digest,str) and re.fullmatch(r'sha256:[0-9a-f]{64}',digest) is not None and not desc.get('urls'), 'invalid/foreign OCI descriptor')
        path = layout/'blobs/sha256'/digest.split(':')[1]
        regular(path)
        require(path.stat().st_size == desc['size'] and sha(path) == digest.split(':')[1], 'missing/corrupt/truncated OCI blob: '+digest)
        seen[digest] = {'digest':digest,'bytes':desc['size'],'mediaType':desc.get('mediaType')}
        return path
    def walk(desc):
        path = blob(desc)
        if desc['mediaType'] == OCI_INDEX:
            identities.add(desc['digest'])
            nested = read(path)
            require(nested.get('schemaVersion')==2,'invalid nested OCI index')
            for child in nested['manifests']: walk(child)
        elif desc['mediaType'] == OCI_MANIFEST:
            identities.add(desc['digest'])
            manifest = read(path)
            require(manifest.get('schemaVersion')==2,'invalid OCI manifest')
            config = read(blob(manifest['config']))
            require(config.get('os')=='linux' and config.get('architecture')=='amd64','wrong OCI SDK platform')
            require(config.get('rootfs',{}).get('type')=='layers','invalid OCI rootfs')
            layers = manifest.get('layers')
            require(isinstance(layers,list) and len(layers)>0,'empty OCI SDK layers')
            require(len(config['rootfs'].get('diff_ids',[])) == len(layers),'OCI diff-id/layer count differs')
            for layer in layers: blob(layer)
            selected.append(desc['digest'])
        else:
            raise ValueError('unsupported OCI index child type')
    for descriptor in index['manifests']:
        walk(descriptor)
    require(expected in identities and len(selected)==1,'expected immutable image identity or unique SDK manifest missing')
    return {'schema_version':1,'index_sha256':sha(layout/'index.json'),'requested_digest':expected,'platform_manifest_digest':selected[0],
            'verified_blobs':sorted(seen.values(),key=lambda v:v['digest']),'claim':'local distribution integrity only; not a signature, SDK content, fresh-builder or executable-equivalence claim'}


def materialize(args):
    integrity = verify_oci(args.layout,args.digest)
    manifest = read(args.layout/'blobs/sha256'/integrity['platform_manifest_digest'].split(':')[1])
    config = read(args.layout/'blobs/sha256'/manifest['config']['digest'].split(':')[1])
    require(len(manifest['layers'])==1, 'minimal scratch SDK must have exactly one COPY layer')
    args.output.mkdir()
    layer = manifest['layers'][0]
    blob = args.layout/'blobs/sha256'/layer['digest'].split(':')[1]
    expected_diff = config['rootfs']['diff_ids'][0]
    require(isinstance(expected_diff,str) and re.fullmatch(r'sha256:[0-9a-f]{64}',expected_diff) is not None,'invalid OCI layer diff-id')
    media = layer.get('mediaType')
    require(media in ('application/vnd.oci.image.layer.v1.tar+gzip','application/vnd.oci.image.layer.v1.tar'),'unsupported SDK layer compression')
    uncompressed = hashlib.sha256()
    class HashedReader:
        def __init__(self,stream): self.stream=stream
        def read(self,size=-1):
            data=self.stream.read(size)
            uncompressed.update(data)
            return data
    total=0
    files=0
    opener=gzip.open if media.endswith('+gzip') else open
    with opener(blob,'rb') as stream:
        reader=HashedReader(stream)
        with tarfile.open(fileobj=reader,mode='r|') as archive:
            for member in archive:
                relative=PurePosixPath(member.name)
                require(not relative.is_absolute() and '..' not in relative.parts and relative.parts and relative.parts[0]=='sdk' and not any(p.startswith('.wh.') for p in relative.parts),'unsafe/non-SDK layer path')
                require(member.isdir() or member.isfile(),'unsupported SDK layer entry type')
                require(member.mode & 0o6000 == 0,'SDK setuid/setgid entry rejected')
                destination=args.output/relative
                if member.isdir():
                    destination.mkdir(parents=True,exist_ok=True)
                    destination.chmod(member.mode & 0o7777)
                else:
                    total+=member.size
                    files+=1
                    require(total <= 8589934592 and files <= 100000,'SDK extraction exceeds 8GiB/100000 files')
                    destination.parent.mkdir(parents=True,exist_ok=True)
                    with destination.open('xb') as output, archive.extractfile(member) as source:
                        shutil.copyfileobj(source,output)
                    destination.chmod(member.mode & 0o7777)
        while reader.read(1048576): pass
    require('sha256:'+uncompressed.hexdigest()==expected_diff,'uncompressed OCI layer diff-id differs')
    sdk=args.output/'sdk'
    producer=read(sdk/'producer.json')
    require(producer['recipe_sha256']==sha(sdk/'recipe.json') and producer['content_sha256']==sha(sdk/'content.json'),'producer recipe/content identities differ')
    locked=read(sdk/'recipe.json')
    require(locked['source']==source_recipe(args.inputs),'materialized SDK recipe inputs differ from requested source')
    content=read(sdk/'content.json')
    require(content.get('schema_version')==1 and content.get('roots')==['glibc-source','native'],'unsupported SDK content roots')
    require(all(PurePosixPath(entry['path']).parts[0] in content['roots'] for entry in content['entries']),'undeclared SDK root')
    require(entries(sdk/'rootfs')==content['entries'],'materialized SDK file/mode/hash inventory differs')
    for name in notice_paths(args.inputs/'scripts/collect-runtime-materials.sh'):
        regular(sdk/'rootfs/native'/name)
    integrity.update({'recipe_sha256':sha(sdk/'recipe.json'),'content_sha256':sha(sdk/'content.json'),'materialized_files':files,'materialized_bytes':total,
                      'claim':'all referenced OCI blob bytes read, layer decompressed/diff-id verified and exact SDK source/content closure materialized; consumer compiler equivalence and fresh-builder linking remain separate'})
    write(args.output/'materialization.json',integrity)
    print(json.dumps(integrity))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action',required=True)
    for action in ('render-producer','render-consumer'):
        cmd = sub.add_parser(action)
        cmd.add_argument('--source',type=Path,required=True)
        cmd.add_argument('--output',type=Path,required=True)
    for action in ('package','verify'):
        cmd = sub.add_parser(action)
        cmd.add_argument('--inputs',type=Path,required=True)
        cmd.add_argument('--native',type=Path,default=Path('/native'))
        cmd.add_argument('--sdk',type=Path,required=True)
        cmd.add_argument('--platform',required=True)
        cmd.add_argument('--debian-snapshot',default='20261017T000000Z')
        cmd.add_argument('--build-cpuset',default='0,1')
        if action=='package':
            cmd.add_argument('--source-commit',required=True)
        else:
            cmd.add_argument('--rootfs',type=Path,required=True)
            cmd.add_argument('--recipe-sha256',required=True)
            cmd.add_argument('--content-sha256',required=True)
    oci = sub.add_parser('verify-oci')
    oci.add_argument('--layout',type=Path,required=True)
    oci.add_argument('--digest',required=True)
    oci.add_argument('--output',type=Path,required=True)
    unpack = sub.add_parser('materialize-oci')
    unpack.add_argument('--layout',type=Path,required=True)
    unpack.add_argument('--digest',required=True)
    unpack.add_argument('--inputs',type=Path,required=True)
    unpack.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    if args.action.startswith('render-'):
        render(args)
    elif args.action=='package':
        package(args)
    elif args.action=='verify':
        verify(args)
    elif args.action=='materialize-oci':
        materialize(args)
    else:
        write(args.output,verify_oci(args.layout,args.digest))


if __name__=='__main__':
    main()
