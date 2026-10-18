#!/usr/bin/env bash
# Parser/identity fixtures only. These are not recorded performance samples.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
helper=$root/scripts/verify-native-profile-samples.sh
owned=$(mktemp -d "${TMPDIR:-/tmp}/indra-profile-samples-XXXXXXXX")
trap 'rm -rf -- "$owned"' EXIT
checks=0

base() {
    printf '# Total Lost Samples: 0\n' > "$owned/report"
    jq -nc 'range(0;64)|{iteration:.,process_id:42}' > "$owned/stats"
    : > "$owned/trace"
    for ((index=0; index<100; index++)); do
        printf ' ocr-profiling 42/%s 123.000001: cpu-clock:u: 0001 entry (ocr-profiling)\n\t0002 work (ocr-profiling)\n\n' "$((1000+index))" >> "$owned/trace"
    done
    printf 'ocr-profiling;entry;work 100\n' > "$owned/folded"
}

positive() {
    local name=$1
    bash "$helper" "$owned/report" "$owned/trace" "$owned/stats" "$owned/folded" "$owned/$name.json"
    jq -e '.accepted==true and .process_id==42 and .samples==100 and .folded_samples==100 and .all_sampled_process_ids_match==true' \
        "$owned/$name.json" > /dev/null
    checks=$((checks+1))
}

negative() {
    local name=$1
    if bash "$helper" "$owned/report" "$owned/trace" "$owned/stats" "$owned/folded" "$owned/$name.json" \
        > "$owned/$name.stdout" 2> "$owned/$name.stderr"; then
        printf 'unexpected profile admission: %s\n' "$name" >&2
        exit 1
    fi
    checks=$((checks+1))
}

base
positive same_process_distinct_threads
base
sed -i '1s/entry/[unknown]/;5s/work/[unknown]/;s/ocr-profiling 42/worker thread 42/' "$owned/trace"
positive bounded_unknown_frames
jq -e '.samples_with_unknown_frames==2' "$owned/bounded_unknown_frames.json" > /dev/null

base
sed -i '1s/42\//43\//' "$owned/trace"
negative foreign_process
base
sed -i '1s/42\/1000/42\/0/' "$owned/trace"
negative invalid_thread
base
sed -i '1s/42\/1000/42/' "$owned/trace"
negative missing_thread
base
sed -i '1s/cpu-clock:u:/cycles:u:/' "$owned/trace"
negative foreign_event
base
printf ' ocr-profiling 43 123.000001: cycles:u: 0001 entry (ocr-profiling)\n' >> "$owned/trace"
negative foreign_event_without_thread
base
printf 'unrecognized record\n' >> "$owned/trace"
negative unrecognized_record
base
sed -i '1s/42\/1000/-42\/1000/' "$owned/trace"
negative negative_process
base
sed -i '1s/42\/1000/42\/4294967296/' "$owned/trace"
negative oversized_thread
base
printf '# metadata only\n' > "$owned/trace"
negative zero_samples
base
sed -i '1,3d' "$owned/trace"
printf 'ocr-profiling;work 99\n' > "$owned/folded"
negative insufficient_samples
jq -e '.accepted==false and .samples==99' "$owned/insufficient_samples.json" > /dev/null

base
jq -c 'if .iteration==3 then .process_id=43 else . end' "$owned/stats" > "$owned/stats.tmp"
mv "$owned/stats.tmp" "$owned/stats"
negative changed_participant
base
jq -c 'del(.process_id)' "$owned/stats" > "$owned/stats.tmp"
mv "$owned/stats.tmp" "$owned/stats"
negative missing_participant
base
sed -i '1d' "$owned/stats"
negative incomplete_requests
base
jq -nc 'range(0;64)|{iteration:0,process_id:42}' > "$owned/stats"
negative duplicate_iteration
base
jq -nc 'range(0;64)|{iteration:.,process_id:4294967296}' > "$owned/stats"
negative oversized_participant

base
printf '# no lost counter\n' > "$owned/report"
negative missing_loss
base
printf '# Total Lost Samples: 0\n# Total Lost Samples: 0\n' > "$owned/report"
negative duplicate_loss
base
printf '# Total Lost Samples: 3\n' > "$owned/report"
negative observed_loss
jq -e '.accepted==false and .lost_samples==3' "$owned/observed_loss.json" > /dev/null
base
printf 'ocr-profiling;work 99\n' > "$owned/folded"
negative folded_mismatch
base
printf 'ocr-profiling;work -1\n' > "$owned/folded"
negative invalid_weight
base
printf '100\n' > "$owned/folded"
negative missing_stack

base
printf 'retained prior observation\n' > "$owned/retained.json"
negative retained
test "$(cat "$owned/retained.json")" = 'retained prior observation'
base
ln -s "$owned/retained.json" "$owned/symlink.json"
negative symlink
test "$(cat "$owned/retained.json")" = 'retained prior observation'
base
truncate -s 1073741825 "$owned/trace"
negative byte_bound

printf '{"scope":"synthetic parser and PID rejection fixtures; no perf measurement","passed":%s,"failed":0}\n' "$checks"
