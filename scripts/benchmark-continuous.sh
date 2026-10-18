#!/usr/bin/env bash
# Frozen issue20 pilot. Native measurement only; missing Tika/math gates stay visible.
set -Eeuo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
protocol=$root/runtime/validation/continuous-protocol.json
source "$root/scripts/lib/cgroup-admission.sh"
docker_cmd() { timeout --signal=TERM --kill-after=5s 30s docker "$@"; }
expand_affinity() {
  local raw=$1 part first last cpu
  local -a parts
  [[ $raw =~ ^[0-9,-]{1,4096}$ ]] || return 1
  IFS=, read -r -a parts <<< "$raw"
  for part in "${parts[@]}"; do
    [[ $part =~ ^([0-9]{1,5})(-([0-9]{1,5}))?$ ]] || return 1
    first=$((10#${BASH_REMATCH[1]}))
    last=${BASH_REMATCH[3]:-${BASH_REMATCH[1]}}; last=$((10#$last))
    ((first<=last && last<=65535)) || return 1
    for ((cpu=first;cpu<=last;cpu++)); do printf '%s\n' "$cpu"; done
  done
}
fail() { printf '%s\n' "$*" >&2; return 1; }
sha() { sha256sum -- "$1" | cut -d ' ' -f 1; }
usage() { printf 'usage: %s plan NEW_OUTPUT | run|run-tika STATIC_BINARY BUILD_IDENTITY_JSON IMAGE_CONFIG_ID MODEL_DIRECTORY NEW_OUTPUT SOURCE_SHA\n' "$0" >&2; exit 2; }
[[ $# -gt 0 ]] || usage
mode=$1; shift
case $mode in
  plan) [[ $# == 1 ]] || usage; output=$1;;
  run|run-tika) [[ $# == 6 ]] || usage; binary=$(realpath -e -- "$1"); metadata=$(realpath -e -- "$2"); image=$3; models=$(realpath -e -- "$4"); output=$5; source_sha=$6;;
  *) usage;;
esac
[[ ! -e $output && ! -L $output ]] || fail 'output must be a new directory'
mkdir -p -- "$output/control" "$output/cells"
output=$(realpath -e -- "$output")
cp -- "$protocol" "$output/control/continuous-protocol.json"
jq -e '.schema_version==1 and .settings.math==false and .settings.default_enabled==false and
  .paired_measurement.repetitions==10 and .paired_measurement.requests_per_repetition==12 and
  .paired_measurement.concurrency==[1,2,3] and .resources.cpus==[1,2] and
  .paired_measurement.input_schedule==["smoke","two-pages","math-page","smoke","math-page","two-pages","two-pages","smoke","math-page","math-page","two-pages","smoke"]' "$protocol" >/dev/null
while IFS=$'\t' read -r path expected; do
  [[ $path != /* && /$path/ != *'/../'* && -f $root/$path && $(sha "$root/$path") == "$expected" ]] || fail 'frozen fixture identity differs'
done < <(jq -r '.fixtures[]|[.path,.sha256]|@tsv' "$protocol")
jq -n --arg hash "$(sha "$protocol")" '
 {schema_version:1,scope:"Prospective issue20 paired implementation pilot; not the frozen quality study",protocol_sha256:$hash,status:"planned",
  cells:[range(0;10) as $repetition|[1,2][] as $cpus|[1,2,3][] as $concurrency|["native","tika"][] as $layer|
    (if ($repetition%2)==0 then ["ordinary","continuous"] else ["continuous","ordinary"] end)[] as $arm|
    {layer:$layer,cpus:$cpus,concurrency:$concurrency,repetition:$repetition,arm:$arm,status:"planned",exit_code:null,summary:null}],
  remaining_gates:["actual native paired cells","complete Tika-layer driver and cells","native numerical/row identity and lifecycle acceptance","separate same-age math compaction experiment","paired engineering-screen analysis"]}' > "$output/study-index.json"
layer=native
[[ $mode != run-tika ]] || layer=tika
if [[ $mode == plan ]]; then
  printf 'Planned 240 cells; no inference, timing, signatures or acceptance claimed.\n'
  exit 0
fi
[[ $image =~ ^sha256:[a-f0-9]{64}$ && $source_sha =~ ^[a-f0-9]{40}$ && -x $binary && -d $models ]] || fail 'immutable image/binary/source/model inputs required'
[[ $(git -C "$root" rev-parse HEAD) == "$source_sha" ]] || fail 'source identity differs from checked out controller'
git -C "$root" diff --quiet HEAD -- runtime scripts/benchmark-continuous.sh scripts/lib/cgroup-admission.sh
jq -e --arg source "$source_sha" --arg binary "$(sha "$binary")" --arg protocol "$(sha "$protocol")" \
  '.schema_version==1 and .source_sha==$source and .binary_sha256==$binary and .protocol_sha256==$protocol and (.native_materials_sha256|type)=="string" and (.native_materials_sha256|test("^[a-f0-9]{64}$")) and .native_equivalence_passed==true and (.native_equivalence_receipt_sha256|test("^[a-f0-9]{64}$"))' "$metadata" >/dev/null
receipt=$(jq -er '.native_equivalence_receipt' "$metadata")
[[ $receipt =~ ^[A-Za-z0-9_.-]+$ && $receipt != . && $receipt != .. ]] || fail 'equivalence receipt must be a regular sibling file'
receipt=$(dirname -- "$metadata")/$receipt
[[ -f $receipt && ! -L $receipt && $(sha "$receipt") == "$(jq -er '.native_equivalence_receipt_sha256' "$metadata")" ]] || fail 'native numerical/lifecycle equivalence receipt differs'
cp -- "$receipt" "$output/control/native-equivalence-receipt"
cp -- "$metadata" "$output/control/build-identity.json"
readelf -W -l "$binary" > "$output/control/binary-program-headers.txt"
readelf -W -d "$binary" > "$output/control/binary-dynamic.txt"
if rg -q 'INTERP' "$output/control/binary-program-headers.txt" || rg -q 'NEEDED' "$output/control/binary-dynamic.txt"; then fail 'experiment must be a static ELF'; fi
[[ -z $(find "$models" -type l -print -quit) ]] || fail 'model directory must contain regular files without symlinks'
(cd "$models" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum) > "$output/control/models.sha256"
lscpu --json > "$output/control/lscpu.json"
cp -- "$root/scripts/benchmark-continuous.sh" "$output/control/controller.sh"
cp -- "$root/scripts/lib/cgroup-admission.sh" "$output/control/cgroup-admission.sh"
cp -- "$root/runtime/validation/host-admission.rs" "$output/control/host-admission.rs"
cp -- "$root/runtime/Cargo.lock" "$output/control/Cargo.lock"
cp -- "$root/backend/models.lock.json" "$output/control/models.lock.json"
sha256sum "$binary" "$protocol" "$metadata" "$receipt" "$root/runtime/Cargo.lock" "$root/backend/models.lock.json" \
  "$root/scripts/benchmark-continuous.sh" "$root/scripts/lib/cgroup-admission.sh" \
  "$root/runtime/validation/host-admission.rs" > "$output/control/source-identities.sha256"
docker_cmd image inspect "$image" > "$output/control/image.json"
jq -e --arg image "$image" 'length==1 and .[0].Id==$image and .[0].Os=="linux" and .[0].Architecture=="amd64"' "$output/control/image.json" >/dev/null
allowed=$(awk '/^Cpus_allowed_list:/ {print $2}' /proc/self/status)
cpus=()
IFS=, read -r -a ranges <<< "$allowed"
for range in "${ranges[@]}"; do
  first=${range%-*}; last=${range#*-}
  [[ $first =~ ^[0-9]+$ && $last =~ ^[0-9]+$ ]] || fail 'invalid process CPU affinity'
  for ((number=first;number<=last && ${#cpus[@]}<2;number++)); do cpus+=("$number"); done
done
[[ ${#cpus[@]} == 2 ]] || fail 'two permitted logical CPUs required'
# Admit two distinct physical cores sharing their L3, then retain raw topology.
for cpu in "${cpus[@]}"; do
  for key in physical_package_id core_id; do
    cat "/sys/devices/system/cpu/cpu$cpu/topology/$key" > "$output/control/cpu-$cpu-$key"
  done
  [[ $(cat "/sys/devices/system/cpu/cpu$cpu/cache/index3/level") == 3 ]]
  cat "/sys/devices/system/cpu/cpu$cpu/cache/index3/shared_cpu_list" > "$output/control/cpu-$cpu-l3"
done
cmp "$output/control/cpu-${cpus[0]}-physical_package_id" "$output/control/cpu-${cpus[1]}-physical_package_id"
! cmp -s "$output/control/cpu-${cpus[0]}-core_id" "$output/control/cpu-${cpus[1]}-core_id"
cmp "$output/control/cpu-${cpus[0]}-l3" "$output/control/cpu-${cpus[1]}-l3"
active='' active_name=''
owner=$(printf '%s\0%s\0%s' "$output" "$source_sha" "$$" | sha256sum | cut -d ' ' -f 1)
cleanup_owned() {
  local id state
  [[ -n $active_name ]] || return 0
  docker_cmd container ls --all --no-trunc --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/control/cleanup-list.txt" || return 1
  id=$(cat "$output/control/cleanup-list.txt")
  if [[ -z $id ]]; then [[ -z $active ]] || return 1; active_name=''; return 0; fi
  [[ $id =~ ^[a-f0-9]{64}$ && ( -z $active || $id == "$active" ) ]] || return 1
  docker_cmd inspect "$id" > "$output/control/cleanup-container.json" || return 1
  jq -e --arg token "$owner" --arg id "$id" --arg image "$image" --arg name "/$active_name" \
    'length==1 and (.[0]|.Id==$id and .Image==$image and .Name==$name and .Config.Labels["io.indra.continuous.owner"]==$token)' \
    "$output/control/cleanup-container.json" >/dev/null || return 1
  state=$(jq -er '.[0].State.Running|tostring' "$output/control/cleanup-container.json")
  if [[ $state == true ]]; then
    docker_cmd stop --timeout 10 "$id" > "$output/control/cleanup-stop.txt" || return 1
    docker_cmd inspect "$id" > "$output/control/cleanup-stopped.json" || return 1
    jq -e --arg id "$id" 'length==1 and .[0].Id==$id and .[0].State.Running==false' "$output/control/cleanup-stopped.json" >/dev/null || return 1
  fi
  docker_cmd rm "$id" > "$output/control/cleanup-removed.txt" || return 1
  docker_cmd container ls --all --no-trunc --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/control/cleanup-remaining.txt" || return 1
  [[ ! -s $output/control/cleanup-remaining.txt ]] || return 1
  active='' active_name=''
}
finish() {
  local code=$? cleanup=0
  trap - EXIT TERM INT
  cleanup_owned || cleanup=1
  [[ $cleanup == 0 ]] || code=1
  jq --argjson code "$code" --argjson cleanup "$cleanup" '.controller_exit=$code|.cleanup_exit=$cleanup|.status="partial"|(.cells[]|select(.status=="running")) += {status:"interrupted",reason:"Controller exited before admission completed; raw evidence retained."}' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  (cd "$output" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS)
  exit "$code"
}
trap finish EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
jq -r --arg layer "$layer" '.cells|to_entries[]|select(.value.layer==$layer)|[.key,.value.cpus,.value.concurrency,.value.repetition,.value.arm]|@tsv' "$output/study-index.json" > "$output/control/assigned-schedule.tsv"
failed=0
while IFS=$'\t' read -r key budget concurrency repetition arm; do
  affinity=${cpus[0]}; [[ $budget == 1 ]] || affinity=${cpus[0]},${cpus[1]}
  relative="cells/${layer}-cpu${budget}-c${concurrency}-r${repetition}-${arm}"
  directory=$output/$relative
  mkdir -p "$directory"
  active_name="indra-continuous-$key-$$"
  docker_cmd container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$directory/preexisting.txt"
  [[ ! -s $directory/preexisting.txt ]] || fail 'owned experiment name already exists'
  jq --argjson key "$key" '.cells[$key].status="running"' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  docker_cmd create --label "io.indra.continuous.owner=$owner" --name "$active_name" --cgroupns=private --entrypoint /experiment --network none --no-healthcheck --read-only \
    --user "$(id -u):$(id -g)" --cpus "$budget" --cpuset-cpus "$affinity" \
    --memory 3g --memory-swap 3g --pids-limit 128 --ulimit stack=8388608:8388608 \
    --cap-drop ALL --security-opt no-new-privileges --tmpfs /tmp:rw,nosuid,nodev,size=64m,mode=1777 \
    --mount "type=bind,src=$binary,dst=/experiment,readonly" --mount "type=bind,src=$models,dst=/models,readonly" \
    --mount "type=bind,src=$directory,dst=/evidence" "$image" \
    --host-admission /evidence/host-admitted --model-dir /models --output /evidence/run --arm "$arm" --concurrency "$concurrency" --cpus "$budget" \
    > "$directory/container-id.txt" 2> "$directory/create.stderr"
  active=$(cat "$directory/container-id.txt")
  [[ $active =~ ^[a-f0-9]{64}$ ]] || fail 'create did not return a full owned CID'
  docker_cmd inspect "$active" > "$directory/container-before.json"
  jq -e --arg id "$active" --arg image "$image" --arg token "$owner" --arg affinity "$affinity" --argjson cpu "$((budget*1000000000))" \
    'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels["io.indra.continuous.owner"]==$token and
     .HostConfig.NanoCpus==$cpu and .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and
     .HostConfig.MemorySwap==3221225472 and .HostConfig.PidsLimit==128 and .HostConfig.ReadonlyRootfs==true and
     .HostConfig.CgroupnsMode=="private" and .HostConfig.Privileged==false and .HostConfig.PidMode=="" and
     .HostConfig.NetworkMode=="none" and .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and
     .HostConfig.SecurityOpt==["no-new-privileges"] and .Config.Entrypoint==["/experiment"])' \
    "$directory/container-before.json" > "$directory/container-admission.txt" || fail 'container settings differ'
  code=0
  timeout --signal=TERM --kill-after=30s 5400s docker start --attach "$active" > "$directory/stdout.log" 2> "$directory/stderr.log" &
  attach_pid=$!
  deadline=$((SECONDS+25)); host_pid=0
  while ((SECONDS<deadline)); do
    timeout --signal=TERM --kill-after=2s 2s docker inspect "$active" > "$directory/container-running.json"
    host_pid=$(jq -er '.[0].State.Pid' "$directory/container-running.json")
    [[ $host_pid == 0 ]] || break
    sleep 0.02
  done
  [[ $host_pid =~ ^[1-9][0-9]*$ ]] || fail 'container never started for host admission'
  cp "/proc/$host_pid/cgroup" "$directory/host-process.cgroup"
  relative_group=$(awk -F: '$1=="0" {print $3}' "$directory/host-process.cgroup")
  [[ $relative_group == /* && $(basename "$relative_group") == "docker-$active.scope" ]] || fail 'host PID is not in its owned Docker cgroup'
  host_group=$(readlink -e "/sys/fs/cgroup$relative_group")
  attest_host_ancestors "$host_group" "$budget" "$directory/host-ancestors.txt"
  expand_affinity "$affinity" > "$directory/expected-cpus.txt"
  expand_affinity "$(cat "$host_group/cpuset.cpus.effective")" > "$directory/effective-cpus.txt"
  cp "/proc/$host_pid/status" "$directory/host-process.status"
  expand_affinity "$(awk '/^Cpus_allowed_list:/ {print $2}' "$directory/host-process.status")" > "$directory/process-cpus.txt"
  [[ $(wc -l < "$directory/expected-cpus.txt") == "$budget" ]] || fail 'requested affinity cardinality differs'
  cmp "$directory/expected-cpus.txt" "$directory/effective-cpus.txt" || fail 'effective cgroup affinity differs'
  cmp "$directory/expected-cpus.txt" "$directory/process-cpus.txt" || fail 'live process scheduler affinity differs'
  printf 'admitted\n' > "$directory/host-admitted.next"
  mv "$directory/host-admitted.next" "$directory/host-admitted"
  wait "$attach_pid" || code=$?
  if [[ $code == 124 || $code == 137 ]]; then
    docker_cmd stop --timeout 10 "$active" > "$directory/timeout-stop.txt" 2>&1 || fail 'timed-out owned container did not stop'
  fi
  docker_cmd inspect "$active" > "$directory/container.json"
  status=failed
  if [[ $code == 0 ]] && jq -e --arg image "$image" --arg affinity "$affinity" --argjson cpu "$((budget*1000000000))" \
    'length==1 and .[0].Image==$image and .[0].HostConfig.NanoCpus==$cpu and .[0].HostConfig.CpusetCpus==$affinity and
     .[0].HostConfig.Memory==3221225472 and .[0].HostConfig.MemorySwap==3221225472 and .[0].HostConfig.PidsLimit==128 and
     .[0].State.ExitCode==0 and .[0].State.OOMKilled==false and .[0].State.Running==false' "$directory/container.json" >/dev/null &&
     jq -e --arg arm "$arm" --argjson cpu "$budget" --argjson c "$concurrency" '.complete==true and .successful_requests==12 and .planned_requests==12 and .arm==$arm and .cpus==$cpu and .concurrency==$c and .cancellation_probe==false' "$directory/run/summary.json" >/dev/null; then status=completed; else failed=1; fi
  if [[ $layer == tika && $status == completed ]]; then
    if ! jq -se --slurpfile expected "$metadata" '
      length==5 and ([.[].role]|sort)==["bridge","gateway","java","native","tika"] and
      all(.[]; . as $asset|($expected[0].role_sha256[$asset.role]|test("^[a-f0-9]{64}$")) and $asset.sha256==$expected[0].role_sha256[$asset.role])
    ' "$directory/run/assets.jsonl" > "$directory/role-identities-verified.txt"; then status=failed; failed=1; fi
  fi
  jq --argjson key "$key" --arg status "$status" --argjson code "$code" --arg path "$relative/run/summary.json" '.cells[$key]+={status:$status,exit_code:$code,summary:$path}' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  cleanup_owned || fail 'owned container cleanup incomplete'
  cp "$output/control/cleanup-removed.txt" "$directory/removed.txt"
  cp "$output/control/cleanup-remaining.txt" "$directory/remaining.txt"
done < "$output/control/assigned-schedule.tsv"
jq --arg layer "$layer" '.status="partial"|.assigned_layer=$layer|.assigned_layer_complete=([.cells[]|select(.layer==$layer)|.status=="completed"]|all)|(.cells[]|select(.layer!=$layer)) += {status:"not_executed",reason:"This layer is a separate controller artifact; never substitute another layer measurement."}' "$output/study-index.json" > "$output/study-index.next.json"
mv -- "$output/study-index.next.json" "$output/study-index.json"
printf '%s cells finished (failure flag=%s); full study remains partial until both layer artifacts and native/math gates are admitted.\n' "$layer" "$failed"
exit 1
