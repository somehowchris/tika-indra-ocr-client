#!/usr/bin/env python3
"""Completion-only independent profiling receipt admission and compact human report.

Read-only reporting tool. Hashes models/binary and rechecks raw sample streams;
do not run during any profiling, timing or compilation window.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import tempfile

LABEL = 'org.tika-indra.native-profile'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_text(), parse_constant=lambda value: (_ for _ in ()).throw(ValueError('nonfinite JSON: '+value)))


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def text(path):
    return path.read_text().strip()


def positive(value):
    return type(value) in (int,float) and math.isfinite(value) and value > 0


def counters(path):
    pairs=[line.split() for line in path.read_text().splitlines()]
    require(all(len(p)==2 and p[1].isdigit() for p in pairs),'invalid resource counter: '+str(path))
    result=dict(pairs)
    require(len(result)==len(pairs),'duplicate resource counter')
    return {key:int(value) for key,value in result.items()}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('study','bundle','models','source'):
        p.add_argument('--'+name,type=Path,required=True)
    for name in ('expected-source','expected-tools-image','expected-protocol-sha256','expected-bundle-manifest-sha256'):
        p.add_argument('--'+name,required=True)
    p.add_argument('--expected-cpus-1',default='0')
    p.add_argument('--expected-cpus-2',default='0,1')
    p.add_argument('--activity-note',required=True,help='Operator disclosure of host/window activity and limitations; retained in the human report.')
    args=p.parse_args()
    affinities={}
    for cpus,raw in ((1,args.expected_cpus_1),(2,args.expected_cpus_2)):
        require(re.fullmatch(r'[0-9]+(?:,[0-9]+)*',raw) is not None,'invalid independently expected affinity')
        values=[int(value) for value in raw.split(',')]
        require(len(values)==cpus and len(set(values))==cpus,'expected affinity cardinality differs')
        affinities[cpus]='\n'.join(map(str,values))
    require(bool(args.activity_note.strip()),'window activity disclosure required')
    # No hashes, directory scans or sample analysis before final controller cleanup.
    done=read(args.study/'completion.json'); status=read(args.study/'status.json')
    require(done.get('completed') is True and done.get('cpus')==[1,2] and done.get('phases_per_cpu')==4 and done.get('mode')=='profile','full two-cap protocol has not completed')
    require(status.get('exit')==0 and status.get('cleanup_exit')==0,'controller or cleanup failed')
    require(re.fullmatch('[0-9a-f]{40}',args.expected_source) is not None,'full source identity required')
    require(re.fullmatch('sha256:[0-9a-f]{64}',args.expected_tools_image) is not None,'immutable tool image required')
    for value in (args.expected_protocol_sha256,args.expected_bundle_manifest_sha256):
        require(re.fullmatch('[0-9a-f]{64}',value) is not None,'full SHA256 identity required')
    require(sha(args.study/'protocol.json')==args.expected_protocol_sha256==sha(args.bundle/'native-profile-protocol.json'),'protocol identity differs')
    require(sha(args.bundle/'artifacts.sha256')==args.expected_bundle_manifest_sha256,'compiled bundle manifest identity differs')
    require(text(args.bundle/'build-evidence/execution-source.txt')==f'source_commit={args.expected_source}\ncargo_profile=profiling\nlto=thin','compiled source/profile identity differs')
    actual_source=subprocess.check_output(['git','-C',str(args.source),'rev-parse','HEAD'],text=True,timeout=10).strip()
    require(actual_source==args.expected_source,'frozen source checkout differs')
    subprocess.run(['git','-C',str(args.source),'diff','--quiet','HEAD','--'],check=True,timeout=10)
    manifest=dict(line.split('  ',1)[::-1] for line in (args.bundle/'artifacts.sha256').read_text().splitlines())
    expected_ocr=manifest.get('./ocr-profiling',manifest.get('ocr-profiling'))
    require(expected_ocr==sha(args.bundle/'ocr-profiling'),'compiled OCR binary differs from bound manifest')
    require(sha(args.source/'runtime/validation/native-profile-protocol.json')==args.expected_protocol_sha256,'source protocol differs')
    models=read(args.source/'backend/models.lock.json')
    for entry in models['files']:
        path=args.models/entry['name']
        require(path.is_file() and not path.is_symlink() and path.stat().st_size==entry['size'] and sha(path)==entry['sha256'],'model identity differs: '+entry['name'])
    for name in ('bundle','relink','fixture'):
        lines=(args.study/(name+'-verification.txt')).read_text().splitlines()
        require(lines and all(line.endswith(': OK') for line in lines),'original manifest admission failed: '+name)
    require((args.study/'host-bundle.inventory').read_bytes()==(args.study/'volume-bundle.inventory').read_bytes(),'private copy mode/type inventory differs')
    require((args.study/'host-bundle-owner.before').read_bytes()==(args.study/'host-bundle-owner.after').read_bytes(),'host bundle owner/mode changed')
    require(not (args.study/'cleanup-volumes-remaining.txt').read_bytes(),'owned volume remains')
    seed=read(args.study/'seed-final.json')[0]
    owner=seed['Config']['Labels'][LABEL]
    require(seed['Image']==args.expected_tools_image and seed['State']['ExitCode']==0 and not seed['State']['Running'] and not seed['State']['OOMKilled'],'seed failure')
    require(text(args.study/'removed-seed.txt')==seed['Id'],'seed removal receipt differs')
    volume=read(args.study/'cleanup-volume.json')[0]
    require(volume['Driver']=='local' and volume['Options'] is None and volume['Labels']=={LABEL:owner},'volume ownership differs')
    require(text(args.study/'cleanup-volume-rm.txt')==volume['Name'],'volume removal receipt differs')
    protocol=read(args.study/'protocol.json');c=protocol['criterion']
    expected=sorted([f'recognition_width_plan/{policy}-b{cap}/{count}' for policy in c['planner_policies'] for cap in c['planner_batch_caps'] for count in c['planner_counts']]+[f'recognition_tensor_width_scan/{arm}/{shape}' for arm in c['scan_arms'] for shape in c['scan_shapes']]+c['additional_cases'])
    require(len(expected)==46 and len(set(expected))==46,'unexpected prospective Criterion contract')
    reports=[]; phase_ids=set()
    with tempfile.TemporaryDirectory(prefix='indra-profile-admission-') as temporary:
        for cpus in (1,2):
            for phase in ('gates','criterion','tiff','math'):
                directory=args.study/f'cpu-{cpus}'/phase
                phase_status=read(directory/'phase-status.json')
                require(phase_status=={'exit':0,'resource_capture_exit':0},'phase failure')
                container=read(directory/'container.json')[0]; before=read(directory/'container-before.json')[0]
                require(container['Id']==before['Id']==text(directory/'removed.txt'),'container removal identity differs')
                require(container['Id'] not in phase_ids,'phase reused a container/cgroup')
                phase_ids.add(container['Id'])
                execution=dict(line.split('  ',1)[::-1] for line in (directory/'execution-identities.sha256').read_text().splitlines())
                for script in ('profile-native.sh','verify-native-profile-samples.sh'):
                    require(execution['/work/scripts/'+script]==sha(args.source/'scripts'/script),'execution script differs from frozen source')
                built=dict(line.split('  ',1)[::-1] for line in (directory/'build-identities.sha256').read_text().splitlines())
                for path in ('usr/bin/perf','usr/bin/rsvg-convert','profile-tools/bin/inferno-collapse-perf','profile-tools/bin/inferno-flamegraph'):
                    require(execution['/'+path]==built[path],'executed tool differs from image build manifest')
                require(built['profile-tools/build-recipe.Dockerfile']==sha(args.source/'docker/profile-tools.Dockerfile'),'tool recipe differs from frozen source')
                require(container['Config']['Labels'][LABEL]==owner and container['Image']==args.expected_tools_image,'phase owner/image differs')
                require(container['State']['ExitCode']==0 and not container['State']['Running'] and not container['State']['OOMKilled'],'phase terminal state differs')
                host=read(directory/'host-admission.json')
                require(host['admitted'] is True and host['container_id']==container['Id'] and host['host_pid']>0,'host PID/cgroup admission differs')
                require(before['HostConfig']['NanoCpus']==cpus*10**9 and before['HostConfig']['Memory']==3221225472 and before['HostConfig']['MemorySwap']==3221225472 and before['HostConfig']['PidsLimit']==128,'Docker resource limits differ')
                for snapshot in ('before','after'):
                    root=directory/snapshot
                    for name,value in [('cpu.max',f'{cpus*100000} 100000'),('memory.max','3221225472'),('memory.swap.max','0'),('pids.max','128'),('stack-soft-kib','8192')]:
                        require(text(root/name)==value,'actual cgroup resource differs: '+name)
                expected_cpus=text(directory/'expected-cpus.txt')
                require(expected_cpus==affinities[cpus]==text(directory/'cgroup-cpus.txt')==text(directory/'process-cpus.txt'),'actual CPU affinity differs from independently expected CPUs')
                require(counters(directory/'before/memory.events')['oom_kill']==counters(directory/'after/memory.events')['oom_kill'],'OOM occurred')
                exits=list(directory.glob('*.exit'))
                require(exits and all(text(path)=='0' for path in exits),'recorded command failed')
                if phase=='criterion':
                    results=[json.loads(line) for line in (directory/'criterion-results.jsonl').read_text().splitlines()]
                    require(sorted(r['benchmark']['full_id'] for r in results)==expected,'46 Criterion case identities differ')
                    new_dirs=list((directory/'criterion').rglob('new'))
                    require(len(new_dirs)==46,'Criterion sample directories differ')
                    for new in new_dirs:
                        sample=read(new/'sample.json');mean=read(new/'estimates.json')['mean']
                        require(all(len(sample[k])==100 and all(positive(x) for x in sample[k]) for k in ('iters','times')),'Criterion samples differ')
                        require(all(positive(x) for x in (mean['point_estimate'],mean['confidence_interval']['lower_bound'],mean['confidence_interval']['upper_bound'])),'Criterion estimate invalid')
                if phase not in ('tiff','math'):
                    continue
                quality=read(directory/'sample-quality.json')
                replay=Path(temporary)/f'{cpus}-{phase}.json'
                subprocess.run(['bash',str(args.source/'scripts/verify-native-profile-samples.sh'),str(directory/'perf-report.txt'),str(directory/'perf-script.txt'),str(directory/'recorded.jsonl'),str(directory/'folded.txt'),str(replay)],check=True,timeout=30,capture_output=True)
                require(read(replay)==quality,'raw stream replay differs from sample receipt')
                require(quality['accepted'] is True and quality['lost_samples']==0 and quality['all_sampled_process_ids_match'] is True and quality['samples']==quality['folded_samples']>0 and quality['event']=='cpu-clock:u' and quality['frequency_hz']==99,'sample loss/PID/event admission failed')
                for block in ('unrecorded-before','recorded','unrecorded-after'):
                    rows=[json.loads(line) for line in (directory/(block+'.jsonl')).read_text().splitlines()]
                    require(len(rows)==64 and [r['iteration'] for r in rows]==list(range(64)),'request count/order differs')
                    require(all(r['threads']==cpus and r['cv_threads']==cpus and r['math']==(phase=='math') and r['diagnostic'] is False and r['batch_size']==64 and r['intra_op_spinning'] is True and r['inter_op_spinning'] is True and positive(r['elapsed_us']) for r in rows),'request settings differ')
                    require((directory/(block+'.txt')).read_bytes()==(args.bundle/'fixtures'/('math-page.expected.txt' if phase=='math' else 'two-pages.expected.txt')).read_bytes(),'fixture output differs')
                reports.append(f'{cpus} CPU / {phase}: {quality["samples"]} samples, zero lost, {quality["samples_with_unknown_frames"]} with unknown frames; PID {quality["process_id"]} matched.')
    print('PASS: completed native profiling receipt admission; source '+args.expected_source)
    print('Protocol SHA256: '+args.expected_protocol_sha256)
    print(f'Model lock verified: {len(models["files"])} files; model revision '+models['revision'])
    print('Criterion: exactly 46 declared cases × 100 finite samples at each CPU cap.')
    print('\n'.join(reports))
    print('All eight phase exits/resources and owned-container/volume cleanup passed. Host bundle owner/mode unchanged.')
    print('Operator window activity: '+args.activity_note)
    print('Scope: warmed synthetic fixtures and Rust microbenchmarks. Visual review remains required; no Tika throughput, OCR quality, causal speedup or cold-start claim.')


if __name__=='__main__':
    main()
