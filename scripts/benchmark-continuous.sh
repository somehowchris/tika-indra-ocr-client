#!/usr/bin/env bash
# Frozen issue20 pilot. Native measurement only; missing Tika/math gates stay visible.
set -Eeuo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
protocol=$root/runtime/validation/continuous-protocol.json
fail() { printf '%s\n' "$*" >&2; return 1; }
sha() { sha256sum -- "$1" | cut -d ' ' -f 1; }
usage() { printf 'usage: %s plan NEW_OUTPUT | run STATIC_BINARY BUILD_IDENTITY_JSON IMAGE_CONFIG_ID MODEL_DIRECTORY NEW_OUTPUT SOURCE_SHA\n' "$0" >&2; exit 2; }
[[ $# -gt 0 ]] || usage
mode=$1; shift
case $mode in
  plan) [[ $# == 1 ]] || usage; output=$1;;
  run) [[ $# == 6 ]] || usage; binary=$(realpath -e -- "$1"); metadata=$(realpath -e -- "$2"); image=$3; models=$(realpath -e -- "$4"); output=$5; source_sha=$6;;
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
if [[ $mode == plan ]]; then
  printf 'Planned 240 cells; no inference, timing, signatures or acceptance claimed.\n'
  exit 0
fi
[[ $image =~ ^sha256:[a-f0-9]{64}$ && $source_sha =~ ^[a-f0-9]{40}$ && -x $binary && -d $models ]] || fail 'immutable image/binary/source/model inputs required'
[[ $(git -C "$root" rev-parse HEAD) == "$source_sha" ]] || fail 'source identity differs from checked out controller'
git -C "$root" diff --quiet HEAD -- runtime scripts/benchmark-continuous.sh
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
docker image inspect "$image" > "$output/control/image.json"
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
active=''
owner=$(printf '%s\0%s\0%s' "$output" "$source_sha" "$$" | sha256sum | cut -d ' ' -f 1)
finish() {
  local code=$?
  trap - EXIT TERM INT
  if [[ -n $active ]]; then
    docker inspect "$active" > "$output/control/interrupted-container.json" 2> "$output/control/interrupted-inspect.stderr" || true
    if jq -e --arg token "$owner" 'length==1 and .[0].Config.Labels["io.indra.continuous.owner"]==$token' "$output/control/interrupted-container.json" >/dev/null 2>&1; then
      docker stop --timeout 10 "$active" > "$output/control/interrupted-stop.stdout" 2> "$output/control/interrupted-stop.stderr" || true
    fi
  fi
  jq --argjson code "$code" '.controller_exit=$code|.status="partial"|(.cells[]|select(.status=="running")) += {status:"interrupted",reason:"Controller exited before admission completed; raw evidence retained."}' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  (cd "$output" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS)
  exit "$code"
}
trap finish EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
jq -r '.cells|to_entries[]|select(.value.layer=="native")|[.key,.value.cpus,.value.concurrency,.value.repetition,.value.arm]|@tsv' "$output/study-index.json" > "$output/control/native-schedule.tsv"
failed=0
while IFS=$'\t' read -r key budget concurrency repetition arm; do
  affinity=${cpus[0]}; [[ $budget == 1 ]] || affinity=${cpus[0]},${cpus[1]}
  relative="cells/native-cpu${budget}-c${concurrency}-r${repetition}-${arm}"
  directory=$output/$relative
  mkdir -p "$directory"
  active="indra-continuous-$key-$$"
  jq --argjson key "$key" '.cells[$key].status="running"' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  docker create --label "io.indra.continuous.owner=$owner" --name "$active" --entrypoint /experiment --network none --no-healthcheck --read-only \
    --user "$(id -u):$(id -g)" --cpus "$budget" --cpuset-cpus "$affinity" \
    --memory 3g --memory-swap 3g --pids-limit 128 --ulimit stack=8388608:8388608 \
    --cap-drop ALL --security-opt no-new-privileges --tmpfs /tmp:rw,nosuid,nodev,size=64m,mode=1777 \
    --mount "type=bind,src=$binary,dst=/experiment,readonly" --mount "type=bind,src=$models,dst=/models,readonly" \
    --mount "type=bind,src=$directory,dst=/evidence" "$image" \
    --model-dir /models --output /evidence/run --arm "$arm" --concurrency "$concurrency" --cpus "$budget" \
    > "$directory/container-id.txt" 2> "$directory/create.stderr"
  code=0
  timeout --signal=TERM --kill-after=30s 5400s docker start --attach "$active" > "$directory/stdout.log" 2> "$directory/stderr.log" || code=$?
  if [[ $code == 124 || $code == 137 ]]; then docker stop --timeout 10 "$active" > "$directory/timeout-stop.txt" 2>&1 || true; fi
  docker inspect "$active" > "$directory/container.json"
  status=failed
  if [[ $code == 0 ]] && jq -e --arg image "$image" --arg affinity "$affinity" --argjson cpu "$((budget*1000000000))" \
    'length==1 and .[0].Image==$image and .[0].HostConfig.NanoCpus==$cpu and .[0].HostConfig.CpusetCpus==$affinity and
     .[0].HostConfig.Memory==3221225472 and .[0].HostConfig.MemorySwap==3221225472 and .[0].HostConfig.PidsLimit==128 and
     .[0].State.ExitCode==0 and .[0].State.OOMKilled==false and .[0].State.Running==false' "$directory/container.json" >/dev/null &&
     jq -e --arg arm "$arm" --argjson cpu "$budget" --argjson c "$concurrency" '.complete==true and .successful_requests==12 and .planned_requests==12 and .arm==$arm and .cpus==$cpu and .concurrency==$c and .cancellation_probe==false' "$directory/run/summary.json" >/dev/null; then status=completed; else failed=1; fi
  jq --argjson key "$key" --arg status "$status" --argjson code "$code" --arg path "$relative/run/summary.json" '.cells[$key]+={status:$status,exit_code:$code,summary:$path}' "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
  docker rm "$active" > "$directory/removed.txt"
  active=''
done < "$output/control/native-schedule.tsv"
jq '.status="partial"|(.cells[]|select(.layer=="tika")) += {status:"not_executed",reason:"The all-process Tika HTTP experiment driver is not implemented in this checkpoint; never substitute native-only results."}' "$output/study-index.json" > "$output/study-index.next.json"
mv -- "$output/study-index.next.json" "$output/study-index.json"
printf 'Native cells finished (failure flag=%s); full study remains partial because Tika and math gates are pending.\n' "$failed"
exit 1
