#!/usr/bin/env python3
import datetime,hashlib,json,os,subprocess
from pathlib import Path
OUT=Path("/evidence")
def require(c,m):
 if not c:raise ValueError(m)
def save(n,v): (OUT/n).write_text(json.dumps(v,indent=2)+"\n")
def digest(p):
 with p.open("rb") as f:return hashlib.file_digest(f,"sha256").hexdigest()
def counters(text):return {k:int(v) for k,v in (line.split() for line in text.splitlines())}
def cg():
 return {n:(Path("/sys/fs/cgroup")/n).read_text() for n in ("cpu.max","cpu.stat","cpuset.cpus.effective","memory.max","memory.swap.max","memory.events","memory.peak","pids.max")}
def ro_mount(path):
 lines=Path("/proc/self/mountinfo").read_text().splitlines()
 return [line for line in lines if line.split()[4]==path]
def run(name,argv,deadline):
 start=datetime.datetime.now(datetime.timezone.utc).isoformat()
 with (OUT/(name+".stdout")).open("xb") as out,(OUT/(name+".stderr")).open("xb") as err:
  p=subprocess.run(argv,stdin=subprocess.DEVNULL,stdout=out,stderr=err,timeout=deadline)
 record={"argv":argv,"started_utc":start,"ended_utc":datetime.datetime.now(datetime.timezone.utc).isoformat(),"exit_code":p.returncode}
 save(name+".command.json",record)
 require(p.returncode==0,name+" failed")
 return record
require(sorted(os.sched_getaffinity(0))==[12,13],"actual affinity differs")
before=cg();save("cgroup-before.json",before)
require(before["cpu.max"].strip()=="200000 100000","actualCPU quota")
require(before["memory.max"].strip()=="536870912" and before["memory.swap.max"].strip()=="0","actual memory/swap")
require(before["pids.max"].strip()=="128","actualPID bound")
events=counters(before["memory.events"]);require(events["oom"]==0 and events["oom_kill"]==0,"initial OOM")
mounts={path:ro_mount(path) for path in ("/","/corpus","/locks/sources.json","/locks/original.json","/indra-evaluate","/control.py")}
save("mounts.json",mounts)
require(all(len(lines)==1 and "ro" in lines[0].split()[5].split(",") for lines in mounts.values()),"root/input mount not read-only")
lock=json.loads(Path("/locks/sources.json").read_bytes());original=json.loads(Path("/locks/original.json").read_bytes())
require(len(lock["assets"])==113 and len(original["assets"])==119,"index shape")
kept={a["path"]:a for a in lock["assets"]};orig={a["path"]:a for a in original["assets"]}
require(len(kept)==113 and len(orig)==119 and all(orig.get(k)==v for k,v in kept.items()),"source row changed")
missing=set(orig)-set(kept)
expected={"metadata/bhl-adaptation.json","metadata/bongabdo-page.html","metadata/eurlex-ai-act-el.html","metadata/eurlex-batteries-es.html","metadata/eurlex-dsa-fr.html","metadata/eurlex-gdpr-de.html"}
require(missing==expected,"exclusion set differs")
require(digest(Path("/indra-evaluate"))=="2b69a6f930dfcafae57d184d286e204ea64ee775f391b74d1d8e0c40cf3de70c","evaluator bytes")
require(digest(Path("/locks/original.json"))=="28303e77944f16f95faf48ddaae321567db1acadb92072d0b5d38fecfdea420d","original lock bytes")
run("version",["/indra-evaluate","--version"],10)
run("static-program-headers",["readelf","--program-headers","/indra-evaluate"],10)
run("static-dynamic-section",["readelf","--dynamic","/indra-evaluate"],10)
require("INTERP" not in (OUT/"static-program-headers.stdout").read_text(),"ELF interpreter")
require("(NEEDED)" not in (OUT/"static-dynamic-section.stdout").read_text(),"dynamic dependency")
argv=["/indra-evaluate","fetch","--include-metadata","--lock","/locks/sources.json","--root","/corpus"]
run("fetch",argv,180)
lines=(OUT/"fetch.stderr").read_text().splitlines()
require(lines==["verified "+a["path"] for a in lock["assets"]],"full ordered113 verified records differ")
require((OUT/"fetch.stdout").read_bytes()==b"","unexpected fetch stdout")
after=cg();save("cgroup-after.json",after)
events_after=counters(after["memory.events"]);require(events_after["oom"]==0 and events_after["oom_kill"]==0,"OOM during verification")
require(all(after[n]==before[n] for n in ("cpu.max","memory.max","memory.swap.max","cpuset.cpus.effective","pids.max")),"resource limits changed")
result={"schema_version":1,"passed":True,"tool_producing_source":"60c849a0f829c7813c5e3fbfb2dc6e907eda1958",
 "evaluator_sha256":digest(Path("/indra-evaluate")),"audit_index_sha256":digest(Path("/locks/sources.json")),
 "original119_index_sha256":digest(Path("/locks/original.json")),"verified_rows":113,"non_metadata_rows":97,"metadata_rows":16,
 "network":"none (outer Docker inspection required)","affinity":sorted(os.sched_getaffinity(0)),
 "source119_rows_modified":False,"inference_executed":False,"OCR_or_model_loading":False,"new119_verification_claimed":False,
 "scope":"Actual unchanged source41 Rust fetch command verifies the complete approved113-row audit index from read-only cache; no performance/accuracy inference."}
save("result.json",result);print(json.dumps(result),flush=True)
