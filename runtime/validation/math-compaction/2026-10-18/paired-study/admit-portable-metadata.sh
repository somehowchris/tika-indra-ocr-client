#!/usr/bin/env bash
# Read only small JSON/text metadata. No models, tensors, ELF or archive hashing.
set -euo pipefail
study=/vol/md1/tika-indra-artifacts/math-compaction-paired-dfef64b-20261018
criterion=/vol/md1/tika-indra-artifacts/math-gather-criterion-dfef64b-20261018
root=/vol/md1/tika-indra-tracing
destination=$root/runtime/validation/math-compaction/2026-10-18/paired-study
[[ ! -e $destination ]]
mkdir "$destination"
cp "$0" "$destination/admit-portable-metadata.sh"
image=$(jq -er '.[0].Id' "$study/image.json")
owner=$(jq -er '.[0].Config.Labels["org.tika-indra.math-compaction"]' "$study/cpu1/reference/container-before.json")
: > "$destination/cell-identities.jsonl"
for cpus in 1 2; do
 affinity=0; effective=0
 if ((cpus==2)); then affinity=0,1; effective=0-1; fi
 for role in reference gate-ordinary gate-compact r{0..9}-{ordinary,compact}; do
  cell=$study/cpu$cpus/$role
  jq -nc --arg role "$role" --argjson cpus "$cpus" --arg affinity "$affinity" --arg effective "$effective" --arg image "$image" --arg owner "$owner" --arg reference "$study/cpu$cpus/reference/tensors" \
   --slurpfile before "$cell/container-before.json" --slurpfile final "$cell/container-final.json" --slurpfile admission "$cell/host-admission.json" \
   --rawfile quota "$cell/resources/cpu.max.before" --rawfile memory "$cell/resources/memory.max.before" --rawfile swap "$cell/resources/memory.swap.max.before" \
   --rawfile pids "$cell/resources/pids.max.before" --rawfile cpuset "$cell/resources/cpuset.cpus.effective.before" --rawfile status "$cell/host-process.status" \
   --rawfile cgroup "$cell/host-process.cgroup" --rawfile oom "$cell/resources/memory.events.after" --rawfile remaining "$cell/remaining.txt" \
   --rawfile attached "$cell/attach-status.txt" --rawfile removed "$cell/removed.txt" '
   def trim: gsub("\\s+$";"");
   $before[0][0] as $b|$final[0][0] as $f|$admission[0] as $a|
   (["--host-admission","/evidence/host-admitted.txt","--model-dir","/models","--resources","/evidence/resources","--threads",($cpus|tostring)] +
    (if $role=="reference" then ["--record-reference","--reference","/evidence/tensors","--output","/evidence/ordinary.txt"]
     else ["--reference","/reference","--arm",($role|split("-")|last)] +
      (if ($role|startswith("gate-")) then ["--gate"] else ["--warmup"] end) end)) as $arguments|
   if $b.Id!=$f.Id or $b.Image!=$image or $f.Image!=$image or $b.Config.Labels["org.tika-indra.math-compaction"]!=$owner or
    $f.Config.Labels["org.tika-indra.math-compaction"]!=$owner or $a.admitted!=true or $a.container_id!=$b.Id or
    ($a.host_pid|type)!="number" or ($a.cgroup|endswith("/docker-"+$b.Id+".scope")|not) or
    ($cgroup|trim|endswith("/docker-"+$b.Id+".scope")|not) or
    $b.Config.User!="65532:65532" or $b.Config.Entrypoint!=["/math-compaction"] or $b.Config.Cmd!=$arguments or
    $b.HostConfig.NanoCpus!=($cpus*1000000000) or $b.HostConfig.CpusetCpus!=$affinity or
    $b.HostConfig.Memory!=3221225472 or $b.HostConfig.MemorySwap!=3221225472 or $b.HostConfig.PidsLimit!=128 or
    $b.HostConfig.CgroupnsMode!="private" or $b.HostConfig.NetworkMode!="none" or $b.HostConfig.ReadonlyRootfs!=true or
    $b.HostConfig.Privileged!=false or $b.HostConfig.PidMode!="" or $b.HostConfig.CapDrop!=["ALL"] or
    ($b.HostConfig.CapAdd//[])!=[] or $b.HostConfig.SecurityOpt!=["no-new-privileges"] or
    ($b.Mounts|any(.Destination=="/models" and .Type=="bind" and .RW==false)|not) or
    (if $role=="reference" then ($b.Mounts|length)!=2 else
      ($b.Mounts|length)!=3 or ($b.Mounts|any(.Destination=="/reference" and .Source==$reference and .RW==false)|not) end) or
    ($quota|trim)!=(($cpus*100000|tostring)+" 100000") or ($memory|trim)!="3221225472" or
    ($swap|trim)!="0" or ($pids|trim)!="128" or ($cpuset|trim)!=$effective or
    ($status|test("(?m)^Cpus_allowed_list:\\s+"+$effective+"$")|not) or
    ($oom|test("(?m)^oom_kill 0$")|not) or $remaining!="" or ($attached|trim)!="0" or ($removed|trim)!=$b.Id or
    $f.State.Running!=false or $f.State.OOMKilled!=false or $f.State.ExitCode!=0 then error("role identity/resource/cleanup admission failed")
   else {cpus:$cpus,role:$role,container_id:$b.Id,image_id:$image,owner_token:$owner,host_pid:$a.host_pid,
    cgroup:$a.cgroup,argv:$arguments,quota:($quota|trim),affinity:($cpuset|trim),memory_max:($memory|trim),swap_max:($swap|trim),
    exit:$f.State.ExitCode,oom_killed:$f.State.OOMKilled,removed:true,identity_resources_passed:true} end
   ' >> "$destination/cell-identities.jsonl"
 done
 for repetition in {0..9}; do
  cp "$study/analysis/pair-cpu$cpus-r$repetition.json" "$destination/pair-cpu$cpus-r$repetition.json"
 done
 for arm in ordinary compact; do
  cp "$study/cpu$cpus/gate-$arm/stdout.jsonl" "$destination/numerical-cpu$cpus-$arm.json"
 done
 cp "$study/cpu$cpus/reference/stdout.jsonl" "$destination/reference-cpu$cpus.json"
 for case in 6-to-4-at-8 4-to-3-at-23 3-to-2-at-27 2-to-1-at-32; do
  data=$criterion/cpu$cpus/criterion/math_self-cache-gather_$case/new
  jq -n --argjson cpus "$cpus" --slurpfile benchmark "$data/benchmark.json" --slurpfile estimates "$data/estimates.json" --slurpfile samples "$data/sample.json" \
   '{cpus:$cpus,benchmark:$benchmark[0],estimates:$estimates[0],raw_samples:$samples[0],scope:"one hot self-cache K or V tensor, same arena reused without reset; excludes other self cache/cross/mask/tokens/views/ORT/page/Tika"}' > "$destination/criterion-cpu$cpus-$case.json"
 done
done
jq -se 'length==46 and all(.[];.identity_resources_passed)' "$destination/cell-identities.jsonl" > "$destination/cell-admission.json"
cp "$study/status.json" "$study/completion.json" "$study/controllers.sha256" "$study/topology.json" "$study/topology-admission.txt" "$study/executed-elf.sha256" "$study/measure-math-compaction.sh" "$study/cgroup-admission.sh" "$destination/"
cp "$study/materials/receipt.json" "$study/materials/receipt.sha256" "$study/materials/protocol.json" "$study/materials/Cargo.lock" "$study/materials/native-static-archives.sha256" "$study/materials/glibc-archives.txt" "$study/materials/native-build-identity.txt" "$study/materials/native-link-inputs.sha256" "$study/materials/models.lock.json" "$study/materials/models.sha256" "$study/materials/cargo-env.sh" "$destination/"
cp "$criterion/results.json" "$destination/criterion-results.json"
cp "$criterion/controller.sh" "$destination/criterion-controller.sh"
cp "$criterion/inner.sh" "$destination/criterion-inner.sh"
cp "$criterion/status.json" "$destination/criterion-status.json"
cp "$criterion/selected-artifact.json" "$criterion/cargo-identities.sha256" "$criterion/producing-source.sha256" "$criterion/native-cargo-env.sha256" "$criterion/rustc.txt" "$criterion/rustflags.txt" "$destination/"
mkdir "$destination/host-before" "$destination/host-after"
for phase in before after; do cp "$study/host-$phase/utc.txt" "$study/host-$phase/loadavg.txt" "$study/host-$phase/proc-stat.txt" "$study/host-$phase/processes.txt" "$destination/host-$phase/"; done
jq '.resource_scope="Fields named full_arm are the recorded snapshot interval: CPU before is read after CLI setup/host admission; CPU after is read after model verification/load, all-session/selected-arm warmup, reference preparation, cohort, reporting and engine teardown, before remaining final snapshots/exit. memory.peak is the cgroup high-water mark at its read. This is not literal process lifetime."|.interpretation_amendment="Clarifies scope only; all raw values and frozen protocol preserved."' "$study/analysis/paired.json" > "$destination/paired.json"
