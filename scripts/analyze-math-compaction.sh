#!/usr/bin/env bash
# Descriptive pairs; resource field names refer to the recorded snapshot interval.
set -euo pipefail
export LC_ALL=C
[[ $# = 2 && $2 =~ ^[0-9a-f]{64}$ ]] || { echo "usage: $0 COMPLETED_PAIRED_STUDY EXPECTED_RECEIPT_SHA256" >&2; exit 2; }
study=$(realpath -e -- "$1")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
materials=$study/materials
sha() { sha256sum -- "$1" | awk '{print $1}'; }
[[ $(sha "$materials/receipt.json") = "$2" && $(cat "$materials/receipt.sha256") = "$2" ]]
files=(protocol.json Cargo.lock native-build-tools.lock native-static-archives.sha256 glibc-archives.txt native-build-identity.txt cargo-env.sh native-link-inputs.sha256 models.lock.json models.sha256 models-verification.log source.tar build.log)
jq -e --argjson names "$(printf '%s\n' "${files[@]}" | jq -Rsc 'split("\n")[:-1]|sort')" '
    .schema_version==1 and .kind=="caller_locked_local_relink_receipt" and
    (.source_commit|test("^[0-9a-f]{40}$")) and (.elf_sha256|test("^[0-9a-f]{64}$")) and
    (.files|keys|sort)==$names and (.files|all(.[];test("^[0-9a-f]{64}$")))
    ' "$materials/receipt.json" >/dev/null
for file in "${files[@]}"; do
    [[ $(sha "$materials/$file") = $(jq -er --arg name "$file" '.files[$name]' "$materials/receipt.json") ]]
done
source_commit=$(jq -er '.source_commit' "$materials/receipt.json")
[[ $(sha "$study/actual-math-compaction") = $(jq -er '.elf_sha256' "$materials/receipt.json") ]]
(cd "$study" && sha256sum --check --strict executed-elf.sha256) >/dev/null
jq -e --arg source "$source_commit" 'length==1 and .[0].Config.Labels["org.opencontainers.image.revision"]==$source' "$study/image.json" >/dev/null
[[ $(git -C "$root" show "$source_commit:runtime/validation/math-compaction-protocol.json" | sha256sum | awk '{print $1}') = $(sha "$materials/protocol.json") ]]
cmp "$materials/protocol.json" "$root/runtime/validation/math-compaction-protocol.json"
jq -e --arg source "$source_commit" '.source==$source' "$study/completion.json" >/dev/null
jq -e '.completed==true and .mode=="paired_timing" and .timed_arms==40' "$study/completion.json" >/dev/null
jq -e '.exit==0 and .cleanup_exit==0 and .mode=="paired_timing"' "$study/status.json" >/dev/null
[[ ! -e $study/analysis && ! -L $study/analysis ]]
mkdir "$study/analysis"
cp -- "$0" "$study/analysis/analyzer.sh"
sha256sum "$study/analysis/analyzer.sh" > "$study/analysis/analyzer.sha256"
counter() {
    awk -v key="$2" '$1==key {if(found++) exit 1; value=$2} END {if(found!=1 || value !~ /^[0-9]+$/) exit 1; print value}' "$1"
}
report() {
    jq -se --arg arm "$2" --argjson cpus "$3" --argjson gate "$4" '
        length==1 and (.[0]|
        .schema_version==1 and .arm==$arm and .threads==$cpus and .iteration==0 and
        .gate==$gate and .timing_valid==($gate|not) and .cohort_ns>=.decoder_ns and .decoder_ns>0 and
        ([.cohort_ns,.decoder_ns]|all(.[];type=="number" and isfinite and floor==.)) and
        .stop_steps==[8,32,8,27,60,23] and
        (if $arm=="ordinary" then .row_steps==360 and .tensor_views==124 and .live_sizes==[6] and
            .gathered_bytes_cross_self_mask_tokens==[0,0,0,0] and .verified_gather_elements==0 and
            .compared_float_elements==(if $gate then 66625920 else 0 end)
         else .row_steps==158 and .tensor_views==288 and .live_sizes==[6,4,3,2,1] and
            .gathered_bytes_cross_self_mask_tokens==[1996800,714240,1200,48] and
            .verified_gather_elements==(if $gate then 678066 else 0 end) and
            .compared_float_elements==(if $gate then 25243936 else 0 end) end))
        ' "$1" >/dev/null
}
finished() {
    jq -e 'length==1 and (.[0].State|.Running==false and .OOMKilled==false and .ExitCode==0)' "$1/container-final.json" >/dev/null
    [[ $(cat "$1/attach-status.txt") = 0 && ! -s $1/remaining.txt ]]
}
for cpus in 1 2; do
    reference="$study/cpu$cpus/reference"
    jq -se 'length==1 and .[0].reference_created==true and .[0].text_bytes==293' "$reference/stdout.jsonl" >/dev/null
    [[ $(wc -c < "$reference/ordinary.txt") = 293 ]]
    finished "$reference"
    for arm in ordinary compact; do
        cell="$study/cpu$cpus/gate-$arm"
        report "$cell/stdout.jsonl" "$arm" "$cpus" true
        finished "$cell"
    done
    for repetition in {0..9}; do
        for arm in ordinary compact; do
            cell="$study/cpu$cpus/r$repetition-$arm"
            report "$cell/stdout.jsonl" "$arm" "$cpus" false
            finished "$cell"
            jq -n --slurpfile report "$cell/stdout.jsonl" \
                --argjson before "$(counter "$cell/resources/cpu.stat.before" usage_usec)" \
                --argjson after "$(counter "$cell/resources/cpu.stat.after" usage_usec)" \
                --argjson throttled_before "$(counter "$cell/resources/cpu.stat.before" throttled_usec)" \
                --argjson throttled_after "$(counter "$cell/resources/cpu.stat.after" throttled_usec)" \
                --argjson peak "$(cat "$cell/resources/memory.peak.after")" '
                if ($report|length)!=1 or $after<$before or $throttled_after<$throttled_before or
                    ($peak|type!="number" or (isfinite|not) or .<0 or floor!=.) then error("invalid arm resource/report evidence")
                else $report[0]+{full_arm_cpu_us:($after-$before),
                    full_arm_throttled_us:($throttled_after-$throttled_before),full_arm_memory_peak_bytes:$peak} end
                ' > "$study/analysis/cpu$cpus-r$repetition-$arm.json"
        done
        jq -n --slurpfile ordinary "$study/analysis/cpu$cpus-r$repetition-ordinary.json" \
            --slurpfile compact "$study/analysis/cpu$cpus-r$repetition-compact.json" \
            --argjson cpus "$cpus" --argjson repetition "$repetition" '
            {cpus:$cpus,repetition:$repetition,
             order:(if $repetition%2==0 then ["ordinary","compact"] else ["compact","ordinary"] end),
             ordinary:$ordinary[0],compact:$compact[0],
             paired:["cohort_ns","decoder_ns","full_arm_cpu_us","full_arm_throttled_us","full_arm_memory_peak_bytes"]|
                map(. as $metric|{metric:$metric,
                    compact_minus_ordinary:($compact[0][$metric]-$ordinary[0][$metric]),
                    compact_divided_by_ordinary:(if $ordinary[0][$metric]>0 then
                        $compact[0][$metric]/$ordinary[0][$metric] else null end)})}
            ' > "$study/analysis/pair-cpu$cpus-r$repetition.json"
    done
done
jq -s '
    def describe: sort as $x|{count:($x|length),min:$x[0],median:(($x[4]+$x[5])/2),max:$x[-1]};
    if length!=20 then error("expected20 complete pairs") else
    {scope:"Synthetic six-row same-age cohort; ten paired repetitions per CPU. Descriptive only; no population CI, page/Tika latency or production promotion.",
     resource_scope:"Fields named full_arm describe the recorded resource snapshot interval, not literal process lifetime. cpu.stat.before is read after CLI setup and host admission; cpu.stat.after is read after model verification/load, all-session and selected-arm warmup, reference preparation, one measured cohort, reporting and engine teardown, before remaining final resource reads and process exit. memory.peak is the cgroup high-water mark when read. Cohort/decoder wall clocks have the separate scope specified by the protocol.",
     pairs:.,profiles:(group_by(.cpus)|map({cpus:.[0].cpus,metrics:
        ([.[].paired[]]|group_by(.metric)|map({metric:.[0].metric,
            difference:(map(.compact_minus_ordinary)|describe),
            ratio:(map(.compact_divided_by_ordinary)|if any(.==null) then null else describe end)}))}))} end
    ' "$study"/analysis/pair-cpu*-r*.json > "$study/analysis/paired.json"
