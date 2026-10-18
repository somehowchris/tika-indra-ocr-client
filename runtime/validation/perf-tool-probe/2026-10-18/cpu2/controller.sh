#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
root=/vol/md1/tika-indra-private-profile
output=/vol/md1/tika-indra-artifacts/profile-perf-probe-6ac9c60-20261018-v6-cpu2
image=sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142
[[ ! -e $output && ! -L $output ]]
mkdir -m0700 "$output"
cp -- "$0" "$output/controller.sh"
mkdir -m0777 "$output/run"
source /vol/md1/tika-indra-ocr/scripts/lib/cgroup-admission.sh
bounded() { local limit=$1; shift; timeout --signal=TERM --kill-after=5s "$limit" "$@"; }
owner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
label=org.tika-indra.perf-tool-probe
name="indra-perf-probe-$owner"
id=''
cleanup() {
 status=$?; trap - EXIT; cleanup_status=0
 if [[ -n $id ]]; then
  if bounded 15s docker inspect "$id" > "$output/cleanup-container.json" 2> "$output/cleanup-inspect.stderr"; then
   if jq -e --arg id "$id" --arg image "$image" --arg owner "$owner" --arg label "$label" 'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels[$label]==$owner)' "$output/cleanup-container.json" >/dev/null; then
    bounded 30s docker stop --timeout 5 "$id" > "$output/cleanup-stop.txt" 2>&1 || cleanup_status=1
    bounded 15s docker rm "$id" > "$output/cleanup-remove.txt" 2>&1 || cleanup_status=1
   else cleanup_status=1; fi
  else cleanup_status=1; fi
 fi
 bounded 15s docker container ls --all --filter "name=^/$name$" --format '{{.ID}}' > "$output/cleanup-remaining.txt" 2> "$output/cleanup-list.stderr" || cleanup_status=1
 [[ ! -s $output/cleanup-remaining.txt ]] || cleanup_status=1
 [[ $status != 0 || $cleanup_status = 0 ]] || status=1
 jq -n --argjson exit "$status" --argjson cleanup "$cleanup_status" '{exit:$exit,cleanup_exit:$cleanup,scope:"perf tool-only GNU busy-loop; no OCR or timing study"}' > "$output/status.json"
 exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cat > "$output/inner.sh" <<'INNER'
set -euo pipefail
export LC_ALL=C
cd /evidence
start=$SECONDS
while [[ ! -f admitted ]]; do ((SECONDS-start<30)); sleep 0.02; done
[[ $(cat admitted) = admitted ]]
(cd / && sha256sum --check --strict /profile-tools/build-identities.sha256) > tools-verification.txt
/usr/bin/perf version --build-options > perf-version.txt
mkfifo control.fifo ack.fifo
cat /proc/$$/limits > process.limits; cat /proc/$$/status > process.status; cat /proc/sys/kernel/perf_event_mlock_kb > perf_event_mlock_kb; /usr/bin/perf record -vv --no-buildid-cache --delay=-1 --event cpu-clock:u --freq 99 --strict-freq \
 --call-graph dwarf,16384 --mmap-pages=32M --max-size=256M --output perf.data \
 --control=fifo:control.fifo,ack.fifo -- /bin/bash -euo pipefail -c '
 exec 3<>control.fifo 4<>ack.fifo
 printf "enable\n" >&3
 read -r -t5 acknowledgement <&4
 [[ $acknowledgement = ack ]]
 status=0
 timeout --signal=TERM --kill-after=1s 1s /bin/bash -c "while :; do :; done" || status=$?
 [[ $status = 124 ]]
 printf "disable\n" >&3
 read -r -t5 acknowledgement <&4
 [[ $acknowledgement = ack ]]
 ' > record.stdout 2> record.stderr
/usr/bin/perf report --stdio --header --show-nr-samples --no-children --percent-limit 0 --sort dso,symbol --input perf.data > perf-report.txt 2> report.stderr
/usr/bin/perf script --header --show-lost-events -F comm,pid,tid,time,event,ip,sym,dso --input perf.data > perf-script.txt 2> script.stderr
[[ -s perf.data && -s perf-report.txt && -s perf-script.txt ]]
grep -q 'cpu-clock:u' perf-script.txt
cat /sys/fs/cgroup/memory.peak > memory.peak; cat /sys/fs/cgroup/memory.events > memory.events; cat /sys/fs/cgroup/cpu.max > cpu.max
INNER
bounded 30s docker create --name "$name" --label "$label=$owner" --pull=never --user=0:0 \
 --cgroupns=private --network=none --read-only --cap-drop=ALL --cap-add=PERFMON --security-opt=no-new-privileges \
 --cpus=2 --cpuset-cpus=0,1 --memory=3g --memory-swap=3g --pids-limit=128 --ulimit stack=8388608:8388608 --ulimit memlock=1140850688:1140850688 \
 --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m --mount "type=bind,src=$output/inner.sh,dst=/probe.sh,readonly" \
 --mount "type=bind,src=$output/run,dst=/evidence" --entrypoint /usr/bin/timeout "$image" --signal=TERM --kill-after=5s 50s \
 /bin/bash /probe.sh > "$output/container.id"
id=$(cat "$output/container.id"); [[ $id =~ ^[0-9a-f]{64}$ ]]
bounded 15s docker inspect "$id" > "$output/container-before.json"
jq -L "$root/scripts/lib" -e --arg id "$id" --arg image "$image" --arg owner "$owner" --arg label "$label" '
 include "profile-capabilities";
 length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels[$label]==$owner and
 .Config.User=="0:0" and .HostConfig.CgroupnsMode=="private" and .HostConfig.NetworkMode=="none" and
 .HostConfig.ReadonlyRootfs and (.HostConfig.Privileged|not) and .HostConfig.PidMode=="" and
 .HostConfig.CapDrop==["ALL"] and profile_capabilities("tiff") and .HostConfig.SecurityOpt==["no-new-privileges"] and
 .HostConfig.NanoCpus==2000000000 and .HostConfig.CpusetCpus=="0,1" and .HostConfig.Memory==3221225472 and
 .HostConfig.MemorySwap==3221225472 and .HostConfig.PidsLimit==128)
 ' "$output/container-before.json" > "$output/container-admission.txt"
bounded 65s docker start --attach "$id" > "$output/stdout.log" 2> "$output/stderr.log" &
attach=$!
start=$SECONDS; pid=0
while ((SECONDS-start<20)); do
 bounded 2s docker inspect "$id" > "$output/container-running.json"
 pid=$(jq -er '.[0].State.Pid' "$output/container-running.json")
 [[ $pid = 0 ]] || break
 sleep 0.02
done
[[ $pid =~ ^[1-9][0-9]*$ ]]
cp "/proc/$pid/cgroup" "$output/host-process.cgroup"
cp "/proc/$pid/status" "$output/host-process.status"
relative=$(awk -F: '$1=="0"{print $3}' "$output/host-process.cgroup")
[[ $(basename "$relative") = "docker-$id.scope" ]]
group=$(readlink -e "/sys/fs/cgroup$relative")
attest_host_ancestors "$group" 2 "$output/host-ancestors.txt"
[[ $(cat "$group/cpuset.cpus.effective") = 0-1 && $(awk '/^Cpus_allowed_list:/{print $2}' "$output/host-process.status") = 0-1 ]]
printf 'admitted\n' > "$output/run/admitted.next"
mv "$output/run/admitted.next" "$output/run/admitted"
wait "$attach"
bounded 15s docker inspect "$id" > "$output/container-final.json"
jq -e --arg id "$id" 'length==1 and (.[0]|.Id==$id and .State.Running==false and .State.ExitCode==0 and .State.OOMKilled==false)' "$output/container-final.json" > "$output/result.txt"
sha256sum "$output/controller.sh" "$output/inner.sh" "$root/scripts/lib/profile-capabilities.jq" > "$output/source-identities.sha256"
