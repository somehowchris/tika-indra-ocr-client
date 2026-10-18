#!/usr/bin/env python3
"""Separate native build-input index. Never alters the six-image candidate schema."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

ROLES={'indra-server':('server-acceptance','indra-server'),'gates/server-unit':('server-acceptance','gates/server-unit'),
       'gates/server-native':('server-acceptance','gates/server-native'),'ocr':('engine-acceptance','ocr'),
       'engine-acceptance':('engine-acceptance','engine-acceptance'),'gates/unit':('engine-acceptance','gates/unit'),
       'gates/native':('engine-acceptance','gates/native')}


def require(value,message):
    if not value:raise ValueError(message)


def sha(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size<=67108864,'invalid bounded build input record')
    return json.loads(path.read_text())


def write(path,value):
    with path.open('x') as stream:json.dump(value,stream,sort_keys=True,indent=2,allow_nan=False);stream.write('\n')


def invocation():
    repo=os.environ['GITHUB_REPOSITORY'];ref=os.environ['GITHUB_REF'];source=os.environ['GITHUB_SHA']
    require(re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+',repo) is not None,'invalid repository')
    require(ref.startswith('refs/tags/v') and re.fullmatch('[0-9a-f]{40}',source) is not None,'trusted v-tag and exact source required')
    workflow=f'{repo}/.github/workflows/release.yml@{ref}'
    require(os.environ['GITHUB_WORKFLOW_REF']==workflow,'unexpected build-input signing workflow')
    run=os.environ.get('NATIVE_BUILD_RELEASE_RUN_ID',os.environ['GITHUB_RUN_ID'])
    attempt=os.environ.get('NATIVE_BUILD_RELEASE_RUN_ATTEMPT',os.environ['GITHUB_RUN_ATTEMPT'])
    require(re.fullmatch('[0-9]+',run) is not None and re.fullmatch('[0-9]+',attempt) is not None,'exact producer run/attempt required')
    require(run==os.environ['GITHUB_RUN_ID'],'producer belongs to a different workflow run')
    return {'repository':repo,'source_sha':source,'source_ref':ref,'release_run_id':run,'release_run_attempt':attempt,
            'workflow_ref':workflow,'workflow_identity':'https://github.com/'+workflow,'oidc_issuer':'https://token.actions.githubusercontent.com'}


def validate(index,expected):
    require(index.get('schema_version')==1 and index.get('kind')=='native-build-inputs','unexpected build-input index')
    require(all(index.get(k)==v for k,v in expected.items()),'producer invocation identity differs')
    require(set(index.get('inputs',{}))=={'sdk','chef'},'build-input set differs')
    prefix='ghcr.io/'+expected['repository'].split('/')[0].lower()+'/tika-indra-native-'
    for role,record in index['inputs'].items():
        require(re.fullmatch('sha256:[0-9a-f]{64}',record.get('digest','')) is not None,'input digest missing')
        require(record.get('reference')==prefix+role+'@'+record['digest'],'unexpected build-input repository/reference')
        for key in ('recipe_sha256','content_sha256','oci_receipt_sha256','spdx_sha256'):
            require(re.fullmatch('[0-9a-f]{64}',record.get(key,'')) is not None,'input identity missing: '+key)
    require(set(index.get('baseline_roles',{}))==set(ROLES),'seven native baseline roles missing')
    for record in index['baseline_roles'].values():
        require(re.fullmatch('[0-9a-f]{64}',record.get('sha256','')) is not None and type(record.get('bytes')) is int and record['bytes']>0,'baseline binary identity invalid')
    require(re.fullmatch('[0-9a-f]{64}',index.get('production_candidate_index_sha256','')) is not None,'six-image baseline binding missing')


def create(source,release,evidence):
    identity=invocation();baseline=read(release)
    require(subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()==identity['source_sha'],'index source checkout differs')
    require(all(baseline.get(k)==v for k,v in identity.items()),'six-image baseline invocation differs')
    require(sorted(r['role'] for r in baseline['images'])==['backend','bridge','gateway','native','tesseract','tika'],'original six-image set differs')
    sdk=read(evidence/'sdk-materialized/materialization.json');chef=read(evidence/'chef-manifests/key.json')
    records={}
    for role,recipe,content,receipt in [('sdk',sdk['recipe_sha256'],sdk['content_sha256'],evidence/'sdk-materialized/materialization.json'),
                                      ('chef',chef['dependency_key_sha256'],sha(evidence/'chef-manifests/content.json'),evidence/'chef-oci-integrity.json')]:
        reference=(evidence/(role+'.reference')).read_text().strip()
        records[role]={'reference':reference,'digest':reference.split('@')[1],'recipe_sha256':recipe,'content_sha256':content,'oci_receipt_sha256':sha(receipt),'spdx_sha256':sha(evidence/(role+'.spdx.json'))}
    roles={}
    for role,(arm,relative) in ROLES.items():
        path=evidence/'baseline'/arm/relative
        require(path.is_file() and not path.is_symlink(),'baseline role absent: '+role)
        roles[role]={'sha256':sha(path),'bytes':path.stat().st_size}
    require(roles['indra-server']['sha256']==sha(release.parents[1]/'bin/indra-server'),'baseline server differs from tested six-image release')
    value={'schema_version':1,'kind':'native-build-inputs',**identity,'inputs':records,'baseline_roles':roles,
           'production_candidate_index_sha256':sha(release),'claims':{'production_candidate_images':6,'fresh_hosted_restore':False,'seven_role_consumer_equivalence':False,'performance':False},
           'source_recipe_scope':'source Dockerfile and native script closure verified by SDK reader; Chef additionally binds actual root/runtime manifests, locks, shared source, configuration and compiler contract'}
    validate(value,identity);write(evidence/'index.json',value)


def files(evidence):
    # Only deliver required metadata and baselines; OCI/cache payloads remain in
    # GHCR by immutable digest and unique source/run/attempt tag. No auto deletion.
    names=['index.json','index.sigstore.json','result.json','production-candidates.json','production-candidates.sigstore.json','sdk.reference','chef.reference','sdk-materialized/materialization.json','sdk-materialized/sdk/recipe.json','sdk-materialized/sdk/content.json','sdk-materialized/sdk/producer.json','chef-oci-integrity.json','chef-manifests/key.json','chef-manifests/content.json','chef-manifests/cook.log','chef-manifests/dummy-cleanup.json']
    for role in ('sdk','chef'):names += [role+'.spdx.json',role+'.sigstore.json',role+'-sbom.sigstore.json',role+'-inputs.sigstore.json']
    result=[]
    for name in names:
        path=evidence/name;require(path.is_file() and not path.is_symlink(),'required build-input packet file missing: '+name)
        result.append({'path':name,'bytes':path.stat().st_size,'sha256':sha(path)})
    return result


def packet(evidence,output):
    require(not output.exists() and not output.is_symlink(),'packet output already exists')
    output.mkdir()
    for entry in files(evidence):
        destination=output/entry['path'];destination.parent.mkdir(parents=True,exist_ok=True)
        shutil.copyfile(evidence/entry['path'],destination)
    # Baseline exported static binaries, original Cargo JSON/selection receipts,
    # fixtures and complete unchanged relink/source materials are required.
    for path in (evidence/'baseline').rglob('*'):require(not path.is_symlink(),'baseline symlink rejected before packet copy')
    shutil.copytree(evidence/'baseline',output/'baseline',symlinks=False)
    hashes=[]
    for path in sorted(output.rglob('*')):
        require(not path.is_symlink(),'packet symlink rejected')
        if path.is_file():hashes.append(sha(path)+'  '+str(path.relative_to(output))+'\n')
    (output/'SHA256SUMS').write_text(''.join(hashes))



def archive_packet(packet,output):
    require(not output.exists() and not output.is_symlink(),'transport output already exists');output.mkdir()
    archive=output/'packet.tar'
    with tarfile.open(archive,'w',format=tarfile.PAX_FORMAT) as tar:
        for path in sorted(packet.rglob('*')):
            require(not path.is_symlink() and (path.is_file() or path.is_dir()),'unsupported transport input')
            info=tar.gettarinfo(str(path),arcname=str(path.relative_to(packet)))
            info.uid=info.gid=0;info.uname=info.gname='';info.mtime=0
            if path.is_file():
                with path.open('rb') as stream:tar.addfile(info,stream)
            else:tar.addfile(info)
    (output/'packet.tar.sha256').write_text(sha(archive)+'  packet.tar\n')


def unpack_packet(transport,output):
    require(not output.exists() and not output.is_symlink(),'packet extraction must be exclusive')
    archive=transport/'packet.tar'
    require(archive.is_file() and not archive.is_symlink() and archive.stat().st_size<=2147483648,'transport archive exceeds2GiB')
    expected=(transport/'packet.tar.sha256').read_text().split()
    require(len(expected)==2 and expected[1]=='packet.tar' and re.fullmatch('[0-9a-f]{64}',expected[0]) and sha(archive)==expected[0],'transport archive digest differs')
    output.mkdir(mode=0o700);seen=set();total=0
    with tarfile.open(archive,'r:') as tar:
        members=tar.getmembers();require(len(members)<=50000,'transport member bound exceeded')
        for member in members:
            parts=Path(member.name).parts
            require(parts and not Path(member.name).is_absolute() and '..' not in parts and member.name not in seen and '\\' not in member.name,'unsafe/duplicate transport member')
            require(member.isfile() or member.isdir(),'transport links/special files forbidden')
            require(member.mode&0o7000==0,'transport special mode forbidden')
            seen.add(member.name);total+=member.size;require(total<=2147483648,'transport uncompressed bound exceeded')
        def filtered(member,destination):
            # data_filter protects paths; retain original ordinary mode bits so
            # GitHub upload/download normalization cannot change static/material gates.
            return tarfile.data_filter(member,destination).replace(mode=member.mode&0o777,uid=None,gid=None,uname=None,gname=None)
        tar.extractall(output,members=members,filter=filtered)


def main():
    p=argparse.ArgumentParser(description=__doc__);sub=p.add_subparsers(dest='action',required=True)
    c=sub.add_parser('create');c.add_argument('--source',type=Path,required=True);c.add_argument('--release',type=Path,required=True);c.add_argument('--evidence',type=Path,required=True)
    v=sub.add_parser('validate');v.add_argument('--index',type=Path,required=True)
    pack=sub.add_parser('packet');pack.add_argument('--evidence',type=Path,required=True);pack.add_argument('--output',type=Path,required=True)
    a=sub.add_parser('archive-packet');a.add_argument('--packet',type=Path,required=True);a.add_argument('--output',type=Path,required=True)
    u=sub.add_parser('unpack-packet');u.add_argument('--transport',type=Path,required=True);u.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    if args.action=='create':create(args.source,args.release,args.evidence)
    elif args.action=='packet':packet(args.evidence,args.output)
    elif args.action=='archive-packet':archive_packet(args.packet,args.output)
    elif args.action=='unpack-packet':unpack_packet(args.transport,args.output)
    else:validate(read(args.index),invocation())


if __name__=='__main__':main()
