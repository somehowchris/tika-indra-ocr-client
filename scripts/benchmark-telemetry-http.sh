#!/usr/bin/env bash
# Separate frozen local-SDK study; timing additionally requires root clearance.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
source "$root/scripts/lib/cgroup-admission.sh"
protocol=$root/runtime/validation/telemetry-http-protocol.json
sha() { sha256sum -- "$1" | cut -d' ' -f1; }
bounded() { local limit=$1; shift; timeout --signal=TERM --kill-after=5s "$limit" "$@"; }
[[ $# -ge 2 ]] || { echo 'usage: benchmark-telemetry-http.sh plan NEW_OUTPUT | correctness|run STATIC_DRIVER IDENTITY_JSON IMAGE MODELS NEW_OUTPUT' >&2; exit 2; }
mode=$1; shift
case $mode in
plan) [[ $# = 1 ]]; output=$(realpath -m -- "$1"); repetitions=10;;
correctness|run) [[ $# = 5 ]]; binary=$(realpath -e -- "$1"); metadata=$(realpath -e -- "$2"); image=$3; models=$(realpath -e -- "$4"); output=$(realpath -m -- "$5"); repetitions=10; [[ $mode != correctness ]] || repetitions=1;;
*) exit 2;;
esac
[[ ! -e $output && ! -L $output ]]
mkdir -p -- "$(dirname "$output")"
mkdir -m0700 -- "$output"
mkdir "$output/control"
cp "$protocol" "$output/control/protocol.json"
jq -e '.schema_version==1 and .repetitions==10 and .cpus==[1,2] and .http.concurrency==2 and .http.requests_per_cohort==12 and .enabled.otlp_export_batch_size==16 and .worker.math==true' "$protocol" > /dev/null
jq -n --arg mode "$mode" --argjson repetitions "$repetitions" --arg protocol "$(sha "$protocol")" '{schema_version:1,mode:$mode,protocol_sha256:$protocol,status:"planned",cells:[ [1,2][] as $cpus | range(0;$repetitions) as $repetition | (if ($repetition%2)==0 then ["off","enabled"] else ["enabled","off"] end)[] as $arm | {cpus:$cpus,repetition:$repetition,arm:$arm,status:"planned"}]}' > "$output/study.json"
[[ $mode != plan ]] || { echo 'Planned40 paired-arm processes plus2 separate golden references; no execution or timings.'; exit 0; }
[[ -x $binary && -d $models && ! -L $metadata ]]
source_sha=$(git -C "$root" rev-parse HEAD)
git -C "$root" diff --quiet HEAD -- runtime shared scripts/benchmark-telemetry-http.sh scripts/lib/cgroup-admission.sh
# The compiled fixtures must still match the prospectively frozen protocol.
while IFS=$'\t' read -r path expected; do
 [[ $path != /* && /$path/ != *'/../'* && -f $root/$path && $(sha "$root/$path") = "$expected" ]]
done < <(jq -r '.fixtures[]|[.path,.sha256]|@tsv' "$protocol")
# Immutable imports preserve the original continuous study exactly: their Git
# tree and blob IDs at archive commit 2193324. The runtime tree is clean (above),
# so HEAD equality is working-tree equality, and no archive object is needed.
[[ $(git -C "$root" rev-parse HEAD:runtime/crates/indra-gateway/examples/continuous_http) = 994a9be5b732fbba0998319aeefdd740d9d88e04 &&
   $(git -C "$root" rev-parse HEAD:runtime/validation/continuous-protocol.json) = 021af575c8412996f3fe254579b55c41fa280962 ]]
jq -e --arg source "$source_sha" --arg binary "$(sha "$binary")" --arg protocol "$(sha "$protocol")" '
 .schema_version==1 and .source_sha==$source and .binary_sha256==$binary and .protocol_sha256==$protocol and
 (.role_sha256|keys|sort)==["bridge","gateway","java","native","tika"] and all(.role_sha256[];test("^[a-f0-9]{64}$")) and
 (.trace_dependencies_sha256|length)==16 and all(.trace_dependencies_sha256[];test("^[a-f0-9]{64}$")) and
 (.native_materials_sha256|test("^[a-f0-9]{64}$")) and .native_equivalence_passed==true and
 (.native_equivalence_receipt_sha256|test("^[a-f0-9]{64}$"))' "$metadata" > /dev/null
receipt=$(jq -er '.native_equivalence_receipt' "$metadata")
[[ $receipt =~ ^[A-Za-z0-9_.-]+$ && $receipt != . && $receipt != .. ]]
receipt=$(dirname "$metadata")/$receipt
[[ -f $receipt && ! -L $receipt && $(sha "$receipt") = "$(jq -er '.native_equivalence_receipt_sha256' "$metadata")" ]]
cp "$receipt" "$output/control/native-equivalence-receipt"
cp "$metadata" "$output/control/build-identity.json"
readelf -W -l "$binary" > "$output/control/driver.program-headers.txt"
readelf -W -d "$binary" > "$output/control/driver.dynamic.txt"
if grep -q INTERP "$output/control/driver.program-headers.txt" || grep -q NEEDED "$output/control/driver.dynamic.txt"; then echo 'Driver is not static GNU ELF' >&2; exit 1; fi
if [[ $mode = run ]]; then
 clearance=${TELEMETRY_TIMING_CLEARANCE:?Root timing clearance file required; correctness mode remains available}
 jq -e --arg source "$source_sha" --arg binary "$(sha "$binary")" --arg protocol "$(sha "$protocol")" --arg metadata "$(sha "$metadata")" --arg image "$(jq -er '.assembled_image_root' "$metadata")" '.source_sha==$source and .binary_sha256==$binary and .protocol_sha256==$protocol and .identity_metadata_sha256==$metadata and .assembled_image_root==$image and .native_gate_receipts_reviewed==true and .compilation_and_other_measurements_finished==true and .affinity_1=="0" and .affinity_2=="0,1"' "$clearance" > /dev/null
 cp "$clearance" "$output/control/timing-clearance.json"
 affinity_1=0; affinity_2=0,1
else
 affinity_1=${TELEMETRY_CORRECTNESS_CPUSET_1:-12}; affinity_2=${TELEMETRY_CORRECTNESS_CPUSET_2:-12,13}
fi
[[ $affinity_1 =~ ^[0-9]+$ && $affinity_2 =~ ^[0-9]+,[0-9]+$ ]]
IFS=, read -r first second <<< "$affinity_2"
[[ $first = "$affinity_1" && $first != "$second" ]]
for cpu in "$first" "$second"; do
 taskset --cpu-list "$cpu" true
 found=''
 for index in /sys/devices/system/cpu/cpu"$cpu"/cache/index*; do
  if [[ $(cat "$index/level") = 3 ]]; then found=$(cat "$index/shared_cpu_list"); break; fi
 done
 [[ -n $found ]]; printf '%s\n' "$found" > "$output/control/cpu-$cpu-l3.txt"
done
cmp "$output/control/cpu-$first-l3.txt" "$output/control/cpu-$second-l3.txt"
lscpu --json > "$output/control/lscpu.json"
uname -a > "$output/control/kernel.txt"
[[ -z $(find "$models" -type l -print -quit) ]]
(cd "$models" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum) > "$output/control/models.sha256"
if [[ $mode = run ]]; then
 jq -e --arg models "$(sha "$output/control/models.sha256")" '.models_inventory_sha256==$models' "$clearance" > /dev/null
fi
bounded 30s docker image inspect "$image" > "$output/control/image.json"
image=$(jq -er 'select(length==1)|.[0]|select(.Os=="linux" and .Architecture=="amd64")|.Id|select(test("^sha256:[0-9a-f]{64}$"))' "$output/control/image.json")
jq -e --arg image "$image" '.assembled_image_root==$image' "$metadata" > /dev/null
bounded 950s bash "$root/scripts/verify-daemon-image.sh" resolve "$image" "$output/control/image-proof"
owner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
label=org.tika-indra.telemetry-study
active='' active_name='' current_cell='' attach_pid=''
cleanup() {
 code=$?; trap - EXIT; cleanup_code=0
 if [[ -n $active_name ]]; then
  if bounded 15s docker inspect "$active_name" > "$output/control/interrupted.json" 2> "$output/control/interrupted.stderr"; then
   if jq -e --arg owner "$owner" --arg label "$label" --arg image "$image" --arg id "$active" 'length==1 and (.[0]|.Config.Labels[$label]==$owner and .Image==$image and ($id=="" or .Id==$id))' "$output/control/interrupted.json" > /dev/null; then
    id=$(jq -er '.[0].Id' "$output/control/interrupted.json")
    bounded 30s docker stop --timeout 10 "$id" > "$output/control/interrupted-stop.txt" 2>&1 || cleanup_code=1
    bounded 15s docker rm "$id" > "$output/control/interrupted-remove.txt" 2>&1 || cleanup_code=1
   else cleanup_code=1; fi
  else
   bounded 15s docker ps --all --quiet --filter "name=^/$active_name$" > "$output/control/interrupted-existing.txt" 2>&1 || cleanup_code=1
   [[ ! -s $output/control/interrupted-existing.txt ]] || cleanup_code=1
  fi
 fi
 if [[ -n $attach_pid ]]; then
  if kill -0 "$attach_pid" 2>/dev/null; then kill -TERM "$attach_pid" 2>/dev/null || cleanup_code=1; fi
  wait "$attach_pid" 2>/dev/null || true
 fi
 [[ $code != 0 || $cleanup_code = 0 ]] || code=1
 if [[ $code != 0 ]]; then
  jq --arg cell "$current_cell" --argjson exit "$code" '.status="failed"|.exit=$exit|(.cells[]|select(.artifact_directory==$cell and .status=="running")).status="failed"|(.references[]?|select(.artifact_directory==$cell and .status=="running")).status="failed"' "$output/study.json" > "$output/study.next.json"
  mv "$output/study.next.json" "$output/study.json"
 fi
 jq -n --argjson exit "$code" --argjson cleanup "$cleanup_code" '{exit:$exit,cleanup_exit:$cleanup}' > "$output/status.json"
 (cd "$output" && find . -type f ! -path ./SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS)
 exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
run_cell() {
 local cpus=$1 arm=$2 directory=$3 goldens=${4:-} affinity=$affinity_1 status=0 host_pid=0 start relative host_group
 [[ $cpus = 1 ]] || affinity=$affinity_2
 mkdir "$directory"
 current_cell=$(basename "$directory")
 jq --arg cell "$current_cell" --arg arm "$arm" --argjson cpus "$cpus" '
 .status="running"|if $arm=="reference" then .references=((.references//[])+[{cpus:$cpus,status:"running",artifact_directory:$cell}])
 else .cells |= map(if .cpus==$cpus and .arm==$arm and .status=="planned" and (.repetition|tostring)==($cell|capture("-r(?<n>[0-9]+)-").n) then .status="running"|.artifact_directory=$cell else . end) end' "$output/study.json" > "$output/study.next.json"
 mv "$output/study.next.json" "$output/study.json"
 active_name="indra-telemetry-$owner-$cpus-$arm-$(basename "$directory")"
 bounded 15s docker ps --all --quiet --filter "name=^/$active_name$" > "$directory/preexisting.txt"
 [[ ! -s $directory/preexisting.txt ]]
 local -a extra=() arguments=()
 if [[ -n $goldens ]]; then extra=(--mount "type=bind,src=$goldens,dst=/goldens,readonly"); arguments=(--goldens /goldens); fi
 bounded 30s docker create --name "$active_name" --label "$label=$owner" --pull=never --entrypoint /experiment \
  --network=none --read-only --cgroupns=private --user "$(id -u):$(id -g)" --cpus "$cpus" --cpuset-cpus "$affinity" \
  --memory=3g --memory-swap=3g --pids-limit=256 --cap-drop=ALL --security-opt=no-new-privileges --ulimit stack=8388608:8388608 \
  --tmpfs '/tmp:rw,nosuid,nodev,size=64m,mode=1777' \
  --mount "type=bind,src=$binary,dst=/experiment,readonly" --mount "type=bind,src=$models,dst=/models,readonly" \
  --mount "type=bind,src=$directory,dst=/evidence" "${extra[@]}" "$image" \
  --model-dir /models --output /evidence/run --arm "$arm" --cpus "$cpus" "${arguments[@]}" > "$directory/container.id" 2> "$directory/create.stderr"
 active=$(cat "$directory/container.id"); [[ $active =~ ^[a-f0-9]{64}$ ]]
 bounded 15s docker inspect "$active" > "$directory/container-before.json"
 jq -e --arg image "$image" --arg owner "$owner" --arg label "$label" --arg affinity "$affinity" --arg user "$(id -u):$(id -g)" --argjson cpus "$cpus" '
 length==1 and (.[0]|.Image==$image and .Config.Labels[$label]==$owner and .Config.User==$user and .HostConfig.CgroupnsMode=="private" and
 .HostConfig.NanoCpus==($cpus*1000000000) and .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and
 .HostConfig.PidsLimit==256 and .HostConfig.Privileged==false and .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and
 .HostConfig.PidMode=="" and .HostConfig.NetworkMode=="none" and .HostConfig.ReadonlyRootfs==true and .HostConfig.SecurityOpt==["no-new-privileges"] and
 .Config.Entrypoint==["/experiment"] and (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608)))' "$directory/container-before.json" > "$directory/resource-admission.txt"
 bounded 30s bash "$root/scripts/verify-daemon-image.sh" bind "$output/control/image-proof" "$directory/container-before.json" "$directory/executed-identity"
 timeout --signal=TERM --kill-after=15s 600s docker start --attach "$active" > "$directory/stdout" 2> "$directory/stderr" &
 attach_pid=$!
 start=$SECONDS
 while ((SECONDS-start<30)); do
  bounded 2s docker inspect "$active" > "$directory/container-running.json"
  host_pid=$(jq -er '.[0].State.Pid' "$directory/container-running.json")
  [[ $host_pid = 0 ]] || break
  sleep 0.05
 done
 [[ $host_pid =~ ^[1-9][0-9]*$ ]]
 cp "/proc/$host_pid/cgroup" "$directory/host-process.cgroup"
 relative=$(awk -F: '$1=="0"{print $3}' "$directory/host-process.cgroup")
 [[ $relative = /* && $(basename "$relative") = "docker-$active.scope" ]]
 host_group=$(readlink -e "/sys/fs/cgroup$relative")
 attest_host_ancestors "$host_group" "$cpus" "$directory/host-ancestors.txt"
 cp "$host_group/cpuset.cpus.effective" "$directory/host-effective-cpus.txt"
 cp "/proc/$host_pid/status" "$directory/host-process.status"
 cp "/proc/$host_pid/limits" "$directory/host-process.limits"
 awk '$1=="Max" && $2=="stack" && $3=="size" {found++; if($4!=8388608 || $5!=8388608) exit 1} END {if(found!=1) exit 1}' "$directory/host-process.limits"
 local allowed expected="$affinity"
 allowed=$(awk '$1=="Cpus_allowed_list:"{print $2}' "$directory/host-process.status")
 if [[ $cpus = 2 && $second = $((first+1)) ]]; then expected="$first-$second"; fi
 [[ $(cat "$directory/host-effective-cpus.txt") = "$expected" && $allowed = "$expected" ]]
 jq -n --arg id "$active" --argjson pid "$host_pid" --arg group "$host_group" '{admitted:true,container_id:$id,host_pid:$pid,cgroup:$group}' > "$directory/host-admission.next.json"
 mv "$directory/host-admission.next.json" "$directory/host-admission.json"
 wait "$attach_pid" || status=$?
 attach_pid=''
 bounded 15s docker inspect "$active" > "$directory/container-after.json"
 jq -e 'length==1 and (.[0].State|.Running==false and .OOMKilled==false and .ExitCode==0)' "$directory/container-after.json" > "$directory/terminal-admission.txt" || status=1
 jq -e --arg protocol "$(sha "$protocol")" --arg arm "$arm" --argjson cpus "$cpus" '.complete==true and .protocol_sha256==$protocol and .arm==$arm and .cpus==$cpus and .child_cleanup_complete==true and (if $arm=="reference" then .planned_requests==0 else .planned_requests==12 and .successful_requests==12 end)' "$directory/run/summary.json" > "$directory/summary-admission.txt" || status=1
 jq -se --slurpfile expected "$metadata" 'length==21 and ([.[]|select(.role!="trace_dependency")|.role]|sort)==["bridge","gateway","java","native","tika"] and all(.[];. as $asset|if .role=="trace_dependency" then $expected[0].trace_dependencies_sha256[($asset.path|split("/")|last)]==$asset.sha256 else $expected[0].role_sha256[$asset.role]==$asset.sha256 end)' "$directory/run/assets.jsonl" > "$directory/role-admission.txt" || status=1
 bounded 15s docker rm "$active" > "$directory/removed.txt"
 active='' active_name=''
 [[ $status = 0 ]] || { echo "Telemetry $mode failed at $directory; all evidence retained" >&2; return "$status"; }
 jq --arg cell "$current_cell" --arg summary "$(sha "$directory/run/summary.json")" '
 (.cells[]|select(.artifact_directory==$cell)) |= (.status="completed"|.summary_sha256=$summary) |
 (.references[]?|select(.artifact_directory==$cell)) |= (.status="completed"|.summary_sha256=$summary)' "$output/study.json" > "$output/study.next.json"
 mv "$output/study.next.json" "$output/study.json"
 current_cell=''
}
for cpus in 1 2; do
 run_cell "$cpus" reference "$output/reference-cpu-$cpus"
 for ((repetition=0;repetition<repetitions;repetition++)); do
  arms=(off enabled); if ((repetition%2)); then arms=(enabled off); fi
  for arm in "${arms[@]}"; do
   cell="$output/cpu-$cpus-r$repetition-$arm"
   run_cell "$cpus" "$arm" "$cell" "$output/reference-cpu-$cpus/run"
  done
 done
done
jq --arg mode "$mode" '.status="completed"|.scope=(if $mode=="correctness" then "correctness only; CPU12/12,13 is not timing evidence" else "descriptive paired whole-stack local SDK study" end)' "$output/study.json" > "$output/study.next.json"
mv "$output/study.next.json" "$output/study.json"
printf '%s\n' "Completed telemetry $mode; successful cohort and fault/export resource scopes stay separate."
