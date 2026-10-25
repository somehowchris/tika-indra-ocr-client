#!/usr/bin/env bash
# Separate prespecified studies; no profiling fallback, hidden retries or generated measurements.
# Without --threads: the 2026-10-18 one-factor ablations over thin and fat bundles.
# With --threads: the Indra 1.1 thread-policy study over one bundle at sysfs-resolved placements.
set -euo pipefail
export LC_ALL=C
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
source_root=$(realpath -e -- "${PROFILE_SOURCE_ROOT:-$root}")
study=ablation name=native-ablation-protocol.json
if [[ ${1:-} = --threads ]]; then shift; study=threads name=native-thread-study-protocol.json; fi
protocol=$root/runtime/validation/$name
cmp "$source_root/runtime/validation/$name" "$protocol"
# The guest-reported topology of each allowed CPU: the input of every placement.
cpu_topology() {
    local sysfs=$1 cpu directory
    shift
    for cpu in "$@"; do
        directory=$sysfs/cpu$cpu/topology
        [[ $cpu =~ ^[0-9]+$ && -f $directory/thread_siblings_list && -f $directory/core_id && -f $directory/physical_package_id ]] \
            || { echo "CPU $cpu reports no sysfs topology" >&2; return 1; }
        jq -n --argjson cpu "$cpu" --rawfile siblings "$directory/thread_siblings_list" \
            --rawfile core "$directory/core_id" --rawfile package "$directory/physical_package_id" \
            '{cpu:$cpu,core_id:($core|rtrimstr("\n")|tonumber),package:($package|rtrimstr("\n")|tonumber),siblings:$siblings}' || return
    done | jq -s .
}
if [[ ${1:-} = --schedule ]]; then
    [[ $# = 1 ]] || exit 2
    jq -f "$root/scripts/native-ablation-schedule.jq" "$protocol"
    exit 0
fi
if [[ ${1:-} = --placements ]]; then
    [[ $study = threads && $# -ge 2 ]] || { echo 'usage: ablate-native-docker.sh --threads --placements SYSFS_CPU_ROOT CPU...' >&2; exit 2; }
    cpu_topology "$2" "${@:3}" | jq -f "$root/scripts/native-thread-placements.jq"
    exit 0
fi
if [[ $study = threads ]]; then
    [[ $# = 4 ]] || { echo 'usage: ablate-native-docker.sh --threads BUNDLE MODELS NEW_OUTPUT TOOLS_IMAGE' >&2; exit 2; }
    thin=$(realpath -e -- "$1"); arms=(thin)
    shift
else
    [[ $# = 5 ]] || { echo 'usage: ablate-native-docker.sh THIN_BUNDLE FAT_BUNDLE MODELS NEW_OUTPUT TOOLS_IMAGE' >&2; exit 2; }
    thin=$(realpath -e -- "$1"); fat=$(realpath -e -- "$2"); arms=(thin fat)
    shift 2
fi
models=$(realpath -e -- "$1")
output=$(realpath -m -- "$2"); image=$3
for arm in "${arms[@]}"; do [[ -d ${!arm} ]]; done
[[ -d $models && ! -e $output && ! -L $output ]]
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
for arm in "${arms[@]}"; do
    bundle=${!arm}
    mkdir "$output/admission-$arm"
    # Bundle admission also binds every runtime/validation/native-*-protocol.json
    # of the checked-out commit to the bundle's relink source archive.
    bash "$source_root/scripts/profile-native.sh" --verify-bundle "$bundle" "$output/admission-$arm" "$arm"
    if [[ $study = ablation ]]; then cmp "$bundle/native-ablation-protocol.json" "$protocol"; fi
done
if [[ $study = ablation ]]; then
    # These exact deterministic archives bind both arms to identical source and native code.
    for path in relink/rust-source.tar.gz relink/native-link-inputs.tar.gz build-evidence/rustc.txt; do
        [[ -s $thin/$path && -s $fat/$path ]]
        cmp "$thin/$path" "$fat/$path"
    done
    [[ $(sha256sum "$thin/ocr-profiling" | cut -d' ' -f1) != $(sha256sum "$fat/ocr-profiling" | cut -d' ' -f1) ]]
fi
cp "$protocol" "$output/protocol.json"
sha256sum "$protocol" "$root/scripts/ablate-native-docker.sh" "$root/scripts/native-ablation-schedule.jq" \
    "$root/scripts/summarize-native-ablations.jq" "$root/scripts/native-ablation-resources.jq" \
    "$root/scripts/native-thread-placements.jq" "$source_root/scripts/profile-native.sh" \
    "$root/scripts/profile-native-docker.sh" > "$output/study-identities.sha256"
jq -f "$root/scripts/native-ablation-schedule.jq" "$protocol" > "$output/schedule.json"
processes=$(jq -r '(.placements // .cpus | length) * (.fixtures | length) * (.comparisons | length) * (.paired_block_order | length) * 2' "$protocol")
iterations=$(jq -er '.iterations_per_process' "$protocol")
jq -e --argjson n "$processes" 'length==$n and ([.[].id]|unique|length)==$n' "$output/schedule.json" > /dev/null
mapfile -t available < <(bash "$source_root/scripts/profile-native.sh" --available-cpus)
[[ ${#available[@]} -ge 2 ]]
if [[ $study = threads ]]; then
    # Fails before any container starts when the runner lacks either CPU pair.
    cpu_topology /sys/devices/system/cpu "${available[@]}" > "$output/topology.json"
    jq -f "$root/scripts/native-thread-placements.jq" "$output/topology.json" > "$output/resource-selection.json"
else
    # Pin the same permitted affinity once for every paired process.
    jq -n --arg one "${PROFILE_CPUSET_1:-${available[0]}}" --arg two "${PROFILE_CPUSET_2:-${available[0]},${available[1]}}" \
        '{"cpu-1":$one,"cpu-2":$two}' > "$output/resource-selection.json"
fi
cpuset() { jq -er --arg role "$1" '.[$role]' "$output/resource-selection.json"; }
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
# Exact native gates at one and two CPUs for every independently linked artifact.
for arm in "${arms[@]}"; do
    for cpus in 1 2; do
        affinity=$(cpuset "cpu-$cpus")
        run_cell "${!arm}" "$models" "$output/gates-$arm-$cpus" "$image" "$((cpus * 1000))" "$affinity" \
            gates 64 "$cpus" "$cpus" true "$arm" "$iterations"
    done
done
: > "$output/processes.jsonl"
while IFS= read -r row; do
    IFS=$'\t' read -r id millicpus role fixture lto cap threads cv spin < <(jq -r '[.id,(.millicpus // (.cpus*1000)),
        (.cpuset // "cpu-\(.cpus)"),.fixture,.settings.lto,.settings.batch_size,(.settings.threads // .cpus),
        (.settings.cv_threads // .cpus),.settings.spinning]|@tsv' <<< "$row")
    bundle=${!lto} affinity=$(cpuset "$role")
    run_cell "$bundle" "$models" "$output/$id" "$image" "$millicpus" "$affinity" "$fixture" "$cap" \
        "$threads" "$cv" "$spin" "$lto" "$iterations"
    phase=$output/$id/cell/ablation
    jq -n --argjson metadata "$row" --argjson iterations "$iterations" --slurpfile requests "$phase/requests.jsonl" \
        --rawfile before "$phase/before/cpu.stat" --rawfile after "$phase/after/cpu.stat" \
        --rawfile peak "$phase/after/memory.peak" \
        --rawfile loop_before "$phase/loop-enable/cpu.stat" --rawfile loop_after "$phase/loop-disable/cpu.stat" \
        --rawfile threads_before "$phase/loop-enable/threads.tsv" --rawfile threads_after "$phase/loop-disable/threads.tsv" \
        --from-file "$root/scripts/native-ablation-resources.jq" >> "$output/processes.jsonl"
done < <(jq -c '.[]' "$output/schedule.json")
jq -s --slurpfile protocol "$protocol" -f "$root/scripts/summarize-native-ablations.jq" "$output/processes.jsonl" > "$output/analysis.json"
scope='native synthetic fixture one-factor ablations; not total Tika'
if [[ $study = threads ]]; then scope='native synthetic fixture thread-policy study; not total Tika'; fi
jq -n --argjson processes "$processes" --argjson requests "$((processes * iterations))" --arg scope "$scope" \
    '{completed:true,processes:$processes,requests:$requests,scope:$scope}' > "$output/completion.json"
