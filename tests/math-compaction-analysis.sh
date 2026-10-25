#!/usr/bin/env bash
# Known synthetic timing/counter values test the analyzer; no model or container runs.
set -euo pipefail
[[ $# = 1 ]] || { echo "usage: $0 ADMITTED_CORRECTNESS_STUDY" >&2; exit 2; }
reference=$(realpath -e -- "$1")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
fixture=$(mktemp -d "$(dirname "$reference")/.analysis-fixture.XXXXXXXX")
trap 'rm -rf -- "$fixture"' EXIT
# Immutable identity/reference files are hard-linked; never modify their bytes.
for file in image.json actual-math-compaction executed-elf.sha256; do ln "$reference/$file" "$fixture/$file"; done
cp -al "$reference/materials" "$fixture/materials"
receipt=$(cat "$fixture/materials/receipt.sha256")
jq '.mode="paired_timing"|.timed_arms=40' "$reference/completion.json" > "$fixture/completion.json"
printf '{"exit":0,"cleanup_exit":0,"mode":"paired_timing"}\n' > "$fixture/status.json"
for cpus in 1 2; do
    mkdir "$fixture/cpu$cpus"
    for role in reference gate-ordinary gate-compact; do
        mkdir "$fixture/cpu$cpus/$role"
        for file in stdout.jsonl container-final.json attach-status.txt remaining.txt; do
            cp "$reference/cpu$cpus/$role/$file" "$fixture/cpu$cpus/$role/$file"
        done
    done
    cp "$reference/cpu$cpus/reference/ordinary.txt" "$fixture/cpu$cpus/reference/ordinary.txt"
    for repetition in {0..9}; do
        for arm in ordinary compact; do
            cell="$fixture/cpu$cpus/r$repetition-$arm"
            mkdir -p "$cell/resources"
            time=10000; cpu=150; peak=200
            if [[ $arm = compact ]]; then time=8000; cpu=145; peak=210; fi
            jq --argjson ns "$((time+repetition))" '.gate=false|.timing_valid=true|.verified_gather_elements=0|.compared_float_elements=0|.cohort_ns=$ns|.decoder_ns=($ns-1000)' \
                "$reference/cpu$cpus/gate-$arm/stdout.jsonl" > "$cell/stdout.jsonl"
            ln "$reference/cpu$cpus/gate-$arm/container-final.json" "$cell/container-final.json"
            printf '0\n' > "$cell/attach-status.txt"
            : > "$cell/remaining.txt"
            printf 'usage_usec 100\nthrottled_usec 0\n' > "$cell/resources/cpu.stat.before"
            printf 'usage_usec %s\nthrottled_usec 0\n' "$cpu" > "$cell/resources/cpu.stat.after"
            printf '%s\n' "$peak" > "$cell/resources/memory.peak.after"
        done
    done
done
bash "$root/scripts/analyze-math-compaction.sh" "$fixture" "$receipt"
jq -e '
    (.pairs|length)==20 and (.profiles|length)==2 and all(.profiles[];
    (.metrics[]|select(.metric=="cohort_ns")|
        .difference=={count:10,min:-2000,median:-2000,max:-2000} and
        .ratio.count==10 and .ratio.min==0.8 and .ratio.max==(8009/10009) and
        .ratio.median==(((8004/10004)+(8005/10005))/2)) and
    (.metrics[]|select(.metric=="full_arm_cpu_us")|.difference.median== -5) and
    (.metrics[]|select(.metric=="full_arm_memory_peak_bytes")|.difference.median==10) and
    (.metrics[]|select(.metric=="full_arm_throttled_us")|.ratio==null))
    ' "$fixture/analysis/paired.json" >/dev/null
reject() {
    rm -rf -- "$fixture/analysis"
    if bash "$root/scripts/analyze-math-compaction.sh" "$fixture" "${1:-$receipt}" > "$fixture/expected-rejection.log" 2>&1; then
        echo 'Invalid synthetic evidence was accepted' >&2
        exit 1
    fi
}
reject 0000000000000000000000000000000000000000000000000000000000000000
mv "$fixture/cpu1/gate-compact/stdout.jsonl" "$fixture/cpu1/gate-compact/saved.jsonl"
reject
# Restore only the fixture hard link; original correctness evidence is untouched.
mv "$fixture/cpu1/gate-compact/saved.jsonl" "$fixture/cpu1/gate-compact/stdout.jsonl"
cell=$fixture/cpu1/r0-compact
cp "$cell/stdout.jsonl" "$fixture/single.jsonl"
cat "$fixture/single.jsonl" >> "$cell/stdout.jsonl"
reject
cp "$fixture/single.jsonl" "$cell/stdout.jsonl"
printf 'usage_usec 99\nthrottled_usec 0\n' > "$cell/resources/cpu.stat.after"
reject
printf 'Math analysis fixture:20 known pairs; exact statistics; wrong receipt, missing gate, duplicate report and decreasing CPU counter rejected. No inference or timing.\n'
