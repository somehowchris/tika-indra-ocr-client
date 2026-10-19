"""Read-only classification; raw executable hashes remain the acceptance gate."""
import argparse,itertools,json,re,struct,subprocess
from pathlib import Path
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--raw',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args();ROOT=args.raw.resolve();OUTPUT=args.output.resolve()

def sections(data):
    off=struct.unpack_from('<Q',data,40)[0]
    width,count,index=struct.unpack_from('<HHH',data,58)
    headers=[struct.unpack_from('<IIQQQQIIQQ',data,off+i*width) for i in range(count)]
    names=data[headers[index][4]:headers[index][4]+headers[index][5]]
    return {names[h[0]:].split(b'\0',1)[0].decode():h for h in headers}

report={}
for role,target in [('engine-acceptance','engine-acceptance'),('ocr','engine-acceptance'),('indra-server','server-acceptance')]:
    files=[ROOT/arm/target/role for arm in ('baseline','candidate')]
    data=[p.read_bytes() for p in files];headers=[sections(d) for d in data]
    def in_rodata(side,address):
        h=headers[side]['.rodata'];return h[3]<=address<h[3]+h[5]
    processes=[subprocess.Popen(['objdump','-d','--no-show-raw-insn',str(p)],stdout=subprocess.PIPE,text=True) for p in files]
    section=None;counts={};other=[];samples=[]
    for a,b in itertools.zip_longest(*(p.stdout for p in processes),fillvalue=''):
        if a.startswith('Disassembly of section '):
            if a!=b:raise ValueError('executable sections do not align')
            section=a.strip().removeprefix('Disassembly of section ').removesuffix(':')
        if a==b:continue
        pa=re.match(r'^\s+([0-9a-f]+):\s+(\S+)\s*(.*?)\s*$',a);pb=re.match(r'^\s+([0-9a-f]+):\s+(\S+)\s*(.*?)\s*$',b)
        if not pa and not pb:continue
        value=counts.setdefault(section,{'changed_instruction_lines':0,'only_rip_displacement_to_rodata':0,'other':0})
        value['changed_instruction_lines']+=1
        def shape(p):
            return (p[1],p[2],re.sub(r'[-+]?0x[0-9a-f]+(?=\(%rip\))','<rip-displacement>',p[3].split('#',1)[0].strip())) if p else None
        addresses=[re.search(r'# (?:0x)?([0-9a-f]+)(?:\s|$)',p[3]) if p else None for p in (pa,pb)]
        if (pa and pb and shape(pa)==shape(pb) and '(%rip)' in pa[3] and '(%rip)' in pb[3]
                and all(addresses) and all(in_rodata(i,int(m[1],16)) for i,m in enumerate(addresses))):
            value['only_rip_displacement_to_rodata']+=1
            if len(samples)<8:samples.append({'section':section,'baseline':a.strip(),'candidate':b.strip()})
        else:
            value['other']+=1
            if len(other)<16:other.append({'section':section,'baseline':a.strip(),'candidate':b.strip()})
    for p in processes:
        if p.wait()!=0:raise ValueError('objdump failed')
    h=headers[0]['.rela.dyn'];changed=[]
    for offset in range(h[4],h[4]+h[5],24):
        before=struct.unpack_from('<QQq',data[0],offset);after=struct.unpack_from('<QQq',data[1],offset)
        if before!=after:changed.append((before,after))
    report[role]={'executable_sections':counts,'bounded_instruction_samples':samples,'unclassified_samples':other,
        'rela_dyn':{'changed_entries':len(changed),'same_location_and_type':all(x[:2]==y[:2] for x,y in changed),
                    'all_changed_addends_target_rodata':all(in_rodata(0,x[2]) and in_rodata(1,y[2]) for x,y in changed),
                    'relocation_types':sorted({x[1]&0xffffffff for x,y in changed})},
        'scope':'Read-only inspection of all executable sections and relocation fields; no rewritten bytes or normalized acceptance gate.'}
OUTPUT.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({role:{'sections':value['executable_sections'],'rela_dyn':value['rela_dyn']} for role,value in report.items()},indent=2))
