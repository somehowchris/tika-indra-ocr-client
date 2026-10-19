#!/usr/bin/env python3
import datetime,hashlib,json,os,pathlib,subprocess,sys,uuid
ROOT=pathlib.Path(__file__).resolve().parent
REPO=pathlib.Path("/vol/md1/tika-indra-corpus-metadata-acceptance")
AUDIT=REPO/"evaluation/audits/main-metadata-recovery-20261019"
IMAGE="sha256:56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239"
BINARY="/vol/md1/tika-indra-artifacts/final-study-tools-60c849a-20261019T094545Z/output/bin/indra-evaluate"
LABEL="corpus113-"+uuid.uuid4().hex
def require(c,m):
 if not c:raise ValueError(m)
def save(n,v):(ROOT/n).write_text(json.dumps(v,indent=2)+"\n")
def run(name,args,deadline=30,check=True):
 started=datetime.datetime.now(datetime.timezone.utc).isoformat()
 p=subprocess.run(args,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=deadline)
 (ROOT/(name+".stdout")).write_bytes(p.stdout);(ROOT/(name+".stderr")).write_bytes(p.stderr)
 save(name+".command.json",{"argv":args,"started_utc":started,"ended_utc":datetime.datetime.now(datetime.timezone.utc).isoformat(),"exit_code":p.returncode})
 if check:require(p.returncode==0,name+" failed")
 return p
def inspect(cid,name):
 r=run(name,["docker","inspect",cid])
 v=json.loads(r.stdout);require(len(v)==1 and v[0]["Id"]==cid,"container identity")
 require(v[0]["Config"]["Labels"].get("org.tika-indra.corpus-proof")==LABEL,"owned label mismatch")
 require(v[0]["Image"]==IMAGE,"image mismatch")
 return v[0]
require(sorted(os.sched_getaffinity(0))==[12,13],"host control affinity")
out=ROOT/"output";out.mkdir(exist_ok=False)
argv=["docker","create","--name",LABEL,"--label","org.tika-indra.corpus-proof="+LABEL,
 "--network","none","--read-only","--cap-drop","ALL","--security-opt","no-new-privileges:true",
 "--cpus","2","--cpuset-cpus","12,13","--memory","512m","--memory-swap","512m","--pids-limit","128",
 "--user",str(os.getuid())+":"+str(os.getgid()),"--env","PYTHONDONTWRITEBYTECODE=1",
 "--mount","type=bind,src=/vol/md1/tika-indra-artifacts/corpus,dst=/corpus,readonly",
 "--mount","type=bind,src="+str(AUDIT/"metadata-audit-sources.json")+",dst=/locks/sources.json,readonly",
 "--mount","type=bind,src="+str(REPO/"evaluation/acquisition/frozen-downloads.json")+",dst=/locks/original.json,readonly",
 "--mount","type=bind,src="+BINARY+",dst=/indra-evaluate,readonly",
 "--mount","type=bind,src="+str(ROOT/"container-control.py")+",dst=/control.py,readonly",
 "--mount","type=bind,src="+str(out)+",dst=/evidence",
 "--entrypoint","python3",IMAGE,"-B","/control.py"]
cid=None;error=None
try:
 created=run("create",argv);cid=created.stdout.decode().strip()
 require(len(cid)==64 and all(c in"0123456789abcdef" for c in cid),"invalid container id")
 before=inspect(cid,"inspect-before");hc=before["HostConfig"]
 require(hc["NetworkMode"]=="none" and hc["ReadonlyRootfs"] and hc["NanoCpus"]==2000000000,"outer isolation")
 require(hc["CpusetCpus"]=="12,13" and hc["Memory"]==536870912 and hc["MemorySwap"]==536870912 and hc["PidsLimit"]==128,"outer resources")
 require(hc["CapDrop"]==["ALL"] and not hc["Privileged"] and not hc["PidMode"],"capability/host namespace")
 require("no-new-privileges:true" in hc["SecurityOpt"],"privilege setting")
 mounts=before["Mounts"];require(len(mounts)==6,"mount count")
 require(all(m["RW"]==(m["Destination"]=="/evidence") for m in mounts),"mount mutability")
 active=run("start-attached",["docker","start","--attach",cid],deadline=240,check=False)
 after=inspect(cid,"inspect-after")
 require(active.returncode==0 and after["State"]["ExitCode"]==0 and not after["State"]["OOMKilled"],"actual container command failed")
 result=json.loads((out/"result.json").read_bytes());require(result["passed"] and result["verified_rows"]==113,"inner verification")
except BaseException as exc:
 error=str(exc)
finally:
 cleanup={"container_id":cid,"owner_label":LABEL,"errors":[]}
 if cid:
  try:
   inspect(cid,"inspect-cleanup")
   removed=run("remove",["docker","rm","--force",cid],deadline=30)
   present=run("absence",["docker","ps","--all","--no-trunc","--filter","id="+cid,"--format","{{.ID}}"])
   require(present.stdout.strip()==b"","owned container still present")
   cleanup.update(removed=True,exit_code=0)
  except BaseException as exc:cleanup["errors"].append(str(exc));cleanup["exit_code"]=1
 else:cleanup.update(removed=False,exit_code=1)
 save("cleanup.json",cleanup)
 success=error is None and cleanup.get("exit_code")==0
 save("host-result.json",{"passed":success,"error":error,"container_id":cid,"image":IMAGE,
  "owner_label":LABEL,"controller_sha256":hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
  "inner_controller_sha256":hashlib.sha256((ROOT/"container-control.py").read_bytes()).hexdigest(),
  "source48_base":"e3e0370dffe284021621590f13e9c3ec20a24df5","tool_source41":"60c849a0f829c7813c5e3fbfb2dc6e907eda1958"})
print(json.dumps({"passed":success,"error":error,"container_id":cid,"cleanup":cleanup}),flush=True)
raise SystemExit(0 if success else 1)
