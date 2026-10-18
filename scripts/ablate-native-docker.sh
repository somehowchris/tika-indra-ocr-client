#!/usr/bin/env bash
# Separate prespecified study; no profiling fallback, hidden retries or generated measurements.
set -euo pipefail
export LC_ALL=C
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
source_root=$(realpath -e -- "${PROFILE_SOURCE_ROOT:-$root}")
protocol=$root/runtime/validation/native-ablation-protocol.json
cmp "$source_root/runtime/validation/native-ablation-protocol.json" "$protocol"
if [[ ${1:-} = --schedule ]]; then
    [[ $# = 1 ]] || exit 2
    jq -f "$root/scripts/native-ablation-schedule.jq" "$protocol"
    exit 0
fi
[[ $# = 5 ]] || { echo 'usage: ablate-native-docker.sh THIN_BUNDLE FAT_BUNDLE MODELS NEW_OUTPUT TOOLS_IMAGE' >&2; exit 2; }
thin=$(realpath -e -- "$1"); fat=$(realpath -e -- "$2"); models=$(realpath -e -- "$3")
output=$(realpath -m -- "$4"); image=$5
[[ -d $thin && -d $fat && -d $models && ! -e $output && ! -L $output ]]
mkdir -p -- "$(dirname "$output")"
mkdir -m 0700 -- "$output"
# Each subprocess owns/cleans its exact container. A signalled master forwards the
# signal to the active controller, then waits for its bounded cleanup receipt.
child=''
finish() {
    local status=$? cleanup=0
    trap - EXIT
    if [[ -n $child ]]; then
        kill -TERM "$child" 2>/dev/null || true
        wait "$child" || cleanup=$?
        child=''
    fi
    jq -n --argjson exit "$status" --argjson child "$cleanup" \
        '{exit:$exit,interrupted_controller_exit:$child}' > "$output/status.json"
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for arm in thin fat; do
    bundle=${!arm}
    mkdir "$output/admission-$arm"
    bash "$source_root/scripts/profile-native.sh" --verify-bundle "$bundle" "$output/admission-$arm" "$arm"
    cmp "$bundle/native-ablation-protocol.json" "$protocol"
done
# These exact deterministic archives bind both arms to identical source and native code.
for path in relink/rust-source.tar.gz relink/native-link-inputs.tar.gz build-evidence/rustc.txt; do
    [[ -s $thin/$path && -s $fat/$path ]]
    cmp "$thin/$path" "$fat/$path"
done
[[ $(sha256sum "$thin/ocr-profiling" | cut -d' ' -f1) != $(sha256sum "$fat/ocr-profiling" | cut -d' ' -f1) ]]
cp "$protocol" "$output/protocol.json"
sha256sum "$protocol" "$root/scripts/ablate-native-docker.sh" "$root/scripts/native-ablation-schedule.jq" \
    "$root/scripts/summarize-native-ablations.jq" "$source_root/scripts/profile-native.sh" \
    "$root/scripts/profile-native-docker.sh" > "$output/study-identities.sha256"
jq -f "$root/scripts/native-ablation-schedule.jq" "$protocol" > "$output/schedule.json"
jq -e 'length==128 and ([.[].id]|unique|length)==128' "$output/schedule.json" > /dev/null
# Pin the same permitted affinity once for every paired process.
mapfile -t available < <(bash "$source_root/scripts/profile-native.sh" --available-cpus)
[[ ${#available[@]} -ge 2 ]]
export PROFILE_CPUSET_1=${PROFILE_CPUSET_1:-${available[0]}}
export PROFILE_CPUSET_2=${PROFILE_CPUSET_2:-${available[0]},${available[1]}}
jq -n --arg one "$PROFILE_CPUSET_1" --arg two "$PROFILE_CPUSET_2" \
    '{cpu_1_affinity:$one,cpu_2_affinity:$two}' > "$output/resource-selection.json"
# Resolve once, so a mutable image tag cannot change between paired processes.
timeout --signal=TERM --kill-after=5s 30s docker image inspect "$image" > "$output/tools-image.json"
image=$(jq -er 'select(length==1)|.[0].Id|select(test("^sha256:[0-9a-f]{64}$"))' "$output/tools-image.json")
run_cell() {
    bash "$root/scripts/profile-native-docker.sh" --ablation-cell "$@" &
    child=$!
    local status=0
    wait "$child" || status=$?
    child=''
    return "$status"
}
# Exact native gates at both quotas for both independently linked artifacts.
for arm in thin fat; do
    for cpus in 1 2; do
        run_cell "${!arm}" "$models" "$output/gates-$arm-$cpus" "$image" "$cpus" gates 64 true "$arm"
    done
done
: > "$output/processes.jsonl"
while IFS= read -r row; do
    IFS=$'\t' read -r id cpus fixture lto cap spin < <(jq -r '[.id,.cpus,.fixture,.settings.lto,.settings.batch_size,.settings.spinning]|@tsv' <<< "$row")
    bundle=${!lto}
    run_cell "$bundle" "$models" "$output/$id" "$image" "$cpus" "$fixture" "$cap" "$spin" "$lto"
    phase=$output/$id/cpu-$cpus/ablation
    jq -n --argjson metadata "$row" --slurpfile requests "$phase/requests.jsonl" \
        --rawfile before "$phase/before/cpu.stat" --rawfile after "$phase/after/cpu.stat" \
        --rawfile peak "$phase/after/memory.peak" '
      def counter($s;$k): $s|split("\n")|map(select(startswith($k+" ")))|
        if length!=1 then error("missing/duplicate cgroup counter") else .[0]|split(" ")[1]|tonumber end;
      ($requests|map(.elapsed_us)|sort) as $x |
      $metadata+{p50_us:$x[15],p95_us:$x[30],p99_us:$x[31],
        warmed_recognitions_per_second:(32000000/($x|add)),
        phase_cpu_usage_us:(counter($after;"usage_usec")-counter($before;"usage_usec")),
        phase_throttled_us:(counter($after;"throttled_usec")-counter($before;"throttled_usec")),
        phase_peak_memory_bytes:($peak|tonumber),request_count:($requests|length),
        valid_columns:($requests|map(.valid_columns)|add),padded_columns:($requests|map(.padded_columns)|add)}
      | if .phase_cpu_usage_us<0 or .phase_throttled_us<0 or .phase_peak_memory_bytes<=0
        then error("invalid resource observation") else . end' >> "$output/processes.jsonl"
done < <(jq -c '.[]' "$output/schedule.json")
jq -s -f "$root/scripts/summarize-native-ablations.jq" "$output/processes.jsonl" > "$output/analysis.json"
jq -n '{completed:true,processes:128,requests:4096,scope:"native synthetic fixture one-factor ablations; not total Tika"}' > "$output/completion.json"
