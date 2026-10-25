#!/usr/bin/env bash
# Admit a completed thread-policy study from its retained files, recompute every
# process summary and the pre-registered decisions, and print the decision table.
set -euo pipefail
export LC_ALL=C
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
[[ $# = 1 && -d $1 ]] || { echo 'usage: admit-native-thread-study.sh EVIDENCE_DIRECTORY' >&2; exit 2; }
run=$(realpath -e -- "$1")
protocol=$root/runtime/validation/native-thread-study-protocol.json
fixtures=$root/runtime/validation/native-profile-protocol.json
fail() { printf 'thread study admission failed: %s\n' "$*" >&2; exit 1; }
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
jq -e '. == {exit:0,interrupted_controller_exit:0}' "$run/status.json" > /dev/null || fail 'the controller did not exit cleanly'
cmp -s "$protocol" "$run/protocol.json" || fail 'the recorded protocol is not the registered one'
iterations=$(jq -er '.iterations_per_process' "$protocol")
jq -f "$root/scripts/native-ablation-schedule.jq" "$protocol" > "$work/schedule.json"
processes=$(jq -r 'length' "$work/schedule.json")
jq -e --argjson n "$processes" --argjson r "$((processes * iterations))" \
    '. == {completed:true,processes:$n,requests:$r,scope:"native synthetic fixture thread-policy study; not total Tika"}' \
    "$run/completion.json" > /dev/null || fail 'the study is incomplete'
jq -e --slurpfile expected "$work/schedule.json" '. == $expected[0]' "$run/schedule.json" > /dev/null || fail 'the schedule differs from the protocol'
jq -f "$root/scripts/native-thread-placements.jq" "$run/topology.json" > "$work/selection.json" || fail 'the recorded topology does not yield the placements'
jq -e --slurpfile expected "$work/selection.json" '. == $expected[0]' "$run/resource-selection.json" > /dev/null \
    || fail 'the selected cpusets differ from the recorded topology'
admit_cell() {
    local directory=$1 phase=$2 millicpus=$3 cpuset=$4
    jq -e '.exit==0 and .cleanup_exit==0' "$directory/status.json" > /dev/null || fail "$directory did not finish cleanly"
    jq -e '.completed==true' "$directory/completion.json" > /dev/null || fail "$directory is incomplete"
    jq -e '.exit==0 and .resource_capture_exit==0' "$phase/phase-status.json" > /dev/null || fail "$phase failed"
    jq -e --argjson millicpus "$millicpus" --arg cpuset "$cpuset" \
        'length==1 and .[0].HostConfig.NanoCpus==($millicpus*1000000) and .[0].HostConfig.CpusetCpus==$cpuset' \
        "$phase/container-before.json" > /dev/null || fail "$phase ran with another quota or cpuset"
    [[ $(cat "$phase/before/cpu.max") = "$((millicpus * 100)) 100000" ]] || fail "$phase has another cgroup quota"
    [[ $(paste -sd, "$phase/cgroup-cpus.txt") = "$cpuset" ]] || fail "$phase has another effective cpuset"
}
for cpus in 1 2; do
    cpuset=$(jq -er --arg role "cpu-$cpus" '.[$role]' "$run/resource-selection.json")
    phase=$run/gates-thin-$cpus/cell/gates
    admit_cell "$run/gates-thin-$cpus" "$phase" "$((cpus * 1000))" "$cpuset"
    for gate in unit native allocations perf-control ocr-cli core-allocations telemetry-allocations model-allocations; do
        [[ $(cat "$phase/$gate.exit") = 0 ]] || fail "native gate $gate failed at $cpus CPU"
    done
done
: > "$work/processes.jsonl"
while IFS= read -r row; do
    IFS=$'\t' read -r id millicpus role fixture threads cv spin cap < <(jq -r \
        '[.id,.millicpus,.cpuset,.fixture,.settings.threads,.settings.cv_threads,.settings.spinning,.settings.batch_size]|@tsv' <<< "$row")
    cpuset=$(jq -er --arg role "$role" '.[$role]' "$run/resource-selection.json")
    phase=$run/$id/cell/ablation
    admit_cell "$run/$id" "$phase" "$millicpus" "$cpuset"
    [[ $(cat "$phase/recognition.exit") = 0 ]] || fail "$id recognition failed"
    grep -Fq -- "--threads $threads --cv-threads $cv --batch-size $cap --intra-op-spinning $spin --inter-op-spinning $spin --warmup --iterations $iterations " \
        "$phase/recognition.command.txt" || fail "$id ran other settings"
    expected=$(jq -er --arg id "$fixture" '.fixtures[]|select(.id==$id)|.expected_sha256' "$fixtures")
    [[ $(sha256sum "$phase/output.txt" | cut -d' ' -f1) = "$expected" ]] || fail "$id output differs from the reference text"
    jq -se --argjson n "$iterations" --argjson t "$threads" --argjson c "$cv" --argjson s "$spin" \
        'length==$n and all(.[]; .threads==$t and .cv_threads==$c and .intra_op_spinning==$s and .inter_op_spinning==$s)' \
        "$phase/requests.jsonl" > /dev/null || fail "$id journal settings differ"
    jq -n --argjson metadata "$row" --argjson iterations "$iterations" --slurpfile requests "$phase/requests.jsonl" \
        --rawfile before "$phase/before/cpu.stat" --rawfile after "$phase/after/cpu.stat" \
        --rawfile peak "$phase/after/memory.peak" \
        --rawfile loop_before "$phase/loop-enable/cpu.stat" --rawfile loop_after "$phase/loop-disable/cpu.stat" \
        --rawfile threads_before "$phase/loop-enable/threads.tsv" --rawfile threads_after "$phase/loop-disable/threads.tsv" \
        -c --from-file "$root/scripts/native-ablation-resources.jq" >> "$work/processes.jsonl" || fail "$id resource counters are invalid"
done < <(jq -c '.[]' "$work/schedule.json")
jq -se --slurpfile recorded "$run/processes.jsonl" '. == $recorded' "$work/processes.jsonl" > /dev/null \
    || fail 'recorded process summaries differ from their raw files'
jq -s --slurpfile protocol "$protocol" -f "$root/scripts/summarize-native-ablations.jq" "$work/processes.jsonl" > "$work/analysis.json"
jq -e --slurpfile recorded "$run/analysis.json" '. == $recorded[0]' "$work/analysis.json" > /dev/null \
    || fail 'recorded analysis differs from the recomputed decisions'
printf 'PASS: %s processes, %s exact recognitions, %s native gate roles\n\n' "$processes" "$((processes * iterations))" 2
jq -r -f "$root/scripts/native-thread-study-table.jq" "$work/analysis.json"
