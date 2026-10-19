#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
root=/vol/md1/tika-indra-tracing
output=/vol/md1/tika-indra-artifacts/math-gather-criterion-dfef64b-20261018
build=/vol/md1/tika-indra-artifacts/math-compaction-bench-dfef64b-20261018
exported=/vol/md1/tika-indra-artifacts/math-compaction-bench-export-dfef64b-20261018
binary=$exported/export/math-gather-criterion
image=sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142
expected=3392fc381c72885db2dcc303b44321468edcb3b20a8182c9fcef9c55a15b0e6a
[[ ! -e $output && ! -L $output && -f $binary && ! -L $binary && $(sha256sum "$binary" | awk '{print $1}') = "$expected" ]]
jq -e '.exit==0 and .cleanup_exit==0' "$exported/status.json" >/dev/null
mkdir -m0700 "$output"
cp "$0" "$output/controller.sh"
source "$root/scripts/lib/cgroup-admission.sh"
cp "$root/scripts/lib/cgroup-admission.sh" "$output/cgroup-admission.sh"
cp "$exported/selected-artifact.json" "$exported/cargo-identities.sha256" "$exported/producing-source.sha256" "$exported/status.json" "$output/"
cp "$build/receipt.json" "$build/builder-image.json" "$build/export/native-archives-verification.txt" "$build/export/native-cargo-env.sha256" "$build/export/rustc.txt" "$build/export/rustflags.txt" "$output/"
bounded() { local limit=$1; shift; timeout --signal=TERM --kill-after=5s "$limit" "$@"; }
host() {
 mkdir "$1" || return
 date -u +%FT%TZ > "$1/utc.txt" || return
 cat /proc/loadavg > "$1/loadavg.txt" || return
 cat /proc/stat > "$1/proc-stat.txt" || return
 ps -e -o pid,ppid,psr,pcpu,pmem,stat,comm > "$1/processes.txt" || return
}
host "$output/host-before"
owner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n'); label=org.tika-indra.math-gather-study
active=''; active_name=''
owned() { jq -e --arg owner "$owner" --arg label "$label" --arg image "$image" --arg id "$active" 'length==1 and (.[0]|.Config.Labels[$label]==$owner and .Image==$image and ($id=="" or .Id==$id))' "$1" >/dev/null; }
cleanup() {
 code=$?; trap - EXIT; cleanup_code=0
 if [[ -n $active_name ]]; then
  if bounded 15s docker inspect "$active_name" > "$output/interrupted.json" 2> "$output/interrupted.stderr"; then
   if owned "$output/interrupted.json"; then
    id=$(jq -er '.[0].Id' "$output/interrupted.json")
    bounded 30s docker stop --timeout 5 "$id" > "$output/interrupted-stop.txt" 2>&1 || cleanup_code=1
    bounded 15s docker rm "$id" > "$output/interrupted-remove.txt" 2>&1 || cleanup_code=1
   else cleanup_code=1; fi
  fi
  bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/remaining.txt" 2> "$output/cleanup-list.stderr" || cleanup_code=1
  [[ ! -s $output/remaining.txt ]] || cleanup_code=1
 fi
 host "$output/host-after" || cleanup_code=1
 [[ $code != 0 || $cleanup_code = 0 ]] || code=1
 jq -n --argjson exit "$code" --argjson cleanup "$cleanup_code" '{exit:$exit,cleanup_exit:$cleanup,scope:"one hot self-cache gather; four cases per CPU budget; no native decoder or Tika claim"}' > "$output/status.json"
 exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cat > "$output/inner.sh" <<'INNER'
set -euo pipefail
export LC_ALL=C CRITERION_HOME=/evidence/criterion
start=$SECONDS
while [[ ! -f /evidence/admitted ]]; do ((SECONDS-start<30)); sleep 0.02; done
[[ ! -L /evidence/admitted && $(stat -c%s /evidence/admitted) = 9 && $(cat /evidence/admitted) = admitted ]]
snapshot() {
 mkdir "/evidence/$1" || return
 for name in cpu.max cpu.stat cpuset.cpus.effective memory.max memory.swap.max memory.peak memory.events pids.max; do cat "/sys/fs/cgroup/$name" > "/evidence/$1/$name" || return; done
 cat /proc/self/status > "/evidence/$1/process.status" || return
 cat /proc/self/limits > "/evidence/$1/process.limits" || return
}
snapshot before
status=0
/usr/bin/timeout --signal=TERM --kill-after=5s 180s /bench --bench --noplot --warm-up-time 1 --measurement-time 3 --sample-size 100 > /evidence/criterion.stdout 2> /evidence/criterion.stderr || status=$?
printf '%s\n' "$status" > /evidence/criterion.exit
snapshot after
exit "$status"
INNER
for cpus in 1 2; do
 directory=$output/cpu$cpus; mkdir -m0777 "$directory"
 affinity=0; effective=0
 if ((cpus==2)); then affinity=0,1; effective=0-1; fi
 active_name="indra-math-gather-$owner-$cpus"
 bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$directory/preexisting.txt"
 [[ ! -s $directory/preexisting.txt ]]
 bounded 30s docker create --name "$active_name" --label "$label=$owner" --pull=never --user=65532:65532 --network=none --read-only --cgroupns=private \
  --cap-drop=ALL --security-opt=no-new-privileges --cpus="$cpus" --cpuset-cpus="$affinity" --memory=3g --memory-swap=3g --pids-limit=128 \
  --ulimit stack=8388608:8388608 --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m \
  --mount "type=bind,src=$binary,dst=/bench,readonly" --mount "type=bind,src=$output/inner.sh,dst=/inner.sh,readonly" \
  --mount "type=bind,src=$directory,dst=/evidence" --entrypoint /usr/bin/timeout "$image" --signal=TERM --kill-after=10s 240s /bin/bash /inner.sh > "$directory/container.id"
 active=$(cat "$directory/container.id"); [[ $active =~ ^[a-f0-9]{64}$ ]]
 bounded 15s docker inspect "$active" > "$directory/container-before.json"
 owned "$directory/container-before.json"
 jq -e --arg affinity "$affinity" --argjson cpus "$cpus" '.[0]|.Config.User=="65532:65532" and .HostConfig.CgroupnsMode=="private" and .HostConfig.NetworkMode=="none" and .HostConfig.ReadonlyRootfs and (.HostConfig.Privileged|not) and .HostConfig.PidMode=="" and .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and .HostConfig.SecurityOpt==["no-new-privileges"] and .HostConfig.NanoCpus==($cpus*1000000000) and .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and .HostConfig.PidsLimit==128 and (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608)) and (.Mounts|length)==3 and (.Mounts|any(.Destination=="/bench" and .RW==false)) and (.Mounts|any(.Destination=="/inner.sh" and .RW==false))' "$directory/container-before.json" > "$directory/admission.txt"
 bounded 260s docker start --attach "$active" > "$directory/stdout.log" 2> "$directory/stderr.log" &
 attach=$!; start=$SECONDS; pid=0
 while ((SECONDS-start<20)); do
  bounded 2s docker inspect "$active" > "$directory/container-running.json"
  owned "$directory/container-running.json"
  pid=$(jq -er '.[0].State.Pid' "$directory/container-running.json")
  [[ $pid = 0 ]] || break
  sleep 0.02
 done
 [[ $pid =~ ^[1-9][0-9]*$ ]]
 cp "/proc/$pid/cgroup" "$directory/host-process.cgroup"
 cp "/proc/$pid/status" "$directory/host-process.status"
 cp "/proc/$pid/limits" "$directory/host-process.limits"
 relative=$(awk -F: '$1=="0"{print $3}' "$directory/host-process.cgroup")
 [[ $(basename "$relative") = "docker-$active.scope" ]]
 group=$(readlink -e "/sys/fs/cgroup$relative")
 attest_host_ancestors "$group" "$cpus" "$directory/host-ancestors.txt"
 [[ $(cat "$group/cpu.max") = "$((cpus*100000)) 100000" && $(cat "$group/memory.max") = 3221225472 && $(cat "$group/memory.swap.max") = 0 && $(cat "$group/pids.max") = 128 ]]
 [[ $(cat "$group/cpuset.cpus.effective") = "$effective" && $(awk '/^Cpus_allowed_list:/{print $2}' "$directory/host-process.status") = "$effective" ]]
 awk '$1=="Max" && $2=="stack" && $3=="size" {found++; if($4!=8388608 || $5!=8388608) exit 1} END{if(found!=1)exit 1}' "$directory/host-process.limits"
 printf 'admitted\n' > "$directory/admitted.next"; mv "$directory/admitted.next" "$directory/admitted"
 attached=0; wait "$attach" || attached=$?
 printf '%s\n' "$attached" > "$directory/attach.exit"
 bounded 15s docker inspect "$active" > "$directory/container-final.json"
 owned "$directory/container-final.json"
 [[ $attached = 0 && $(cat "$directory/criterion.exit") = 0 ]]
 jq -e '.[0].State|.Running==false and .OOMKilled==false and .ExitCode==0' "$directory/container-final.json" >/dev/null
 bounded 15s docker rm "$active" > "$directory/removed.txt"
 bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$directory/remaining.txt"
 [[ ! -s $directory/remaining.txt ]]
 active=''; active_name=''
done
jq -n --arg elf "$expected" '{completed:true,cpus:[1,2],cases_per_cpu:4,elf_sha256:$elf,source:"dfef64b97b26d729e69d835dd71ba045bd166b97",scope:"single-thread hot self-cache gather/control at four observed shrink steps"}' > "$output/completion.json"
