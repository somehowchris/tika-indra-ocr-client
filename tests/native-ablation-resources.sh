#!/usr/bin/env bash
# Real Linux newline-terminated counters and invalid evidence, no OCR or compiler.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
jq -nc 'range(32)|{elapsed_us:100,valid_columns:80,padded_columns:96,stage_us:[1,2,3,4,5,6,7]}' > "$work/requests"
printf 'usage_usec 10\nuser_usec 6\nsystem_usec 4\nnr_periods 0\nnr_throttled 0\nthrottled_usec 1\n' > "$work/before"
printf 'usage_usec 20\nuser_usec 12\nsystem_usec 8\nnr_periods 1\nnr_throttled 0\nthrottled_usec 1\n' > "$work/loop-before"
printf 'usage_usec 84\nuser_usec 60\nsystem_usec 24\nnr_periods 9\nnr_throttled 2\nthrottled_usec 3\n' > "$work/loop-after"
printf 'usage_usec 130\nuser_usec 100\nsystem_usec 30\nnr_periods 12\nnr_throttled 2\nthrottled_usec 5\n' > "$work/after"
printf '7\tocr-profiling\t10\t2\n8\tocr-profiling\t5\t0\n' > "$work/threads-before"
printf '7\tocr-profiling\t42\t3\n8\tocr-profiling\t9\t1\n9\tocr-profiling\t6\t2\n' > "$work/threads-after"
run() {
 jq -n --argjson metadata '{"id":"real-counter-fixture"}' --argjson iterations "${1:-32}" --slurpfile requests "$work/requests" \
  --rawfile before "$work/before" --rawfile after "$work/after" --rawfile peak "$work/peak" \
  --rawfile loop_before "$work/loop-before" --rawfile loop_after "$work/loop-after" \
  --rawfile threads_before "$work/threads-before" --rawfile threads_after "$work/threads-after" \
  --from-file "$root/scripts/native-ablation-resources.jq"
}
rejected() {
 if run > "$work/bad.stdout" 2> "$work/bad.stderr"; then echo "$1 admitted" >&2; exit 1; fi
}
printf '131923968\n' > "$work/peak"
run > "$work/result"
# A thread started inside the loop counts in full: 32+1, 4+1 and 6+2 switches.
jq -e '.request_count==32 and .phase_peak_memory_bytes==131923968 and .phase_cpu_usage_us==120 and .phase_throttled_us==4 and .p50_us==100 and
  .stage_p50_us==[1,2,3,4,5,6,7] and .loop_cpu_usage_us==64 and .loop_cpu_user_us==48 and .loop_cpu_system_us==16 and
  .loop_periods==8 and .loop_throttled_periods==2 and .loop_throttled_us==2 and .loop_threads==3 and
  .loop_voluntary_switches==42 and .loop_involuntary_switches==4 and .loop_cpu_us_per_request==2 and
  .loop_switches_per_request==(46/32)' "$work/result" > /dev/null
for invalid in '' '0\n' '-1\n' 'NaN\n' '1\n2\n' '1\n\n' '3221225473\n'; do
 printf '%b' "$invalid" > "$work/peak"
 rejected 'invalid peak'
done
printf '131923968\n' > "$work/peak"
cp "$work/after" "$work/after.good"
printf 'usage_usec 130\nusage_usec 140\nthrottled_usec 5\n' > "$work/after"
rejected 'duplicate CPU counter'
printf 'usage_usec 9\nthrottled_usec 5\n' > "$work/after"
rejected 'decreasing CPU counter'
printf 'usage_usec 80\nuser_usec 60\nsystem_usec 20\nnr_periods 9\nnr_throttled 2\nthrottled_usec 5\n' > "$work/after"
rejected 'a phase snapshot before the loop end'
cp "$work/after.good" "$work/after"
cp "$work/threads-after" "$work/threads-after.good"
printf '7\tocr-profiling\t42\t3\n9\tocr-profiling\t6\t2\n' > "$work/threads-after"
rejected 'a thread that exited inside the loop'
printf '7\tocr-profiling\t9\t3\n8\tocr-profiling\t9\t1\n' > "$work/threads-after"
rejected 'a decreasing thread switch counter'
printf '7\tocr-profiling\t42\t3\n7\tocr-profiling\t9\t1\n' > "$work/threads-after"
rejected 'a duplicate thread'
printf '7\tocr-profiling\t42\n' > "$work/threads-after"
rejected 'a truncated thread record'
cp "$work/threads-after.good" "$work/threads-after"
cp "$work/loop-after" "$work/loop-after.good"
cp "$work/loop-before" "$work/loop-after"
rejected 'an empty measured loop'
cp "$work/loop-after.good" "$work/loop-after"
jq -nc 'range(32)|{elapsed_us:100,valid_columns:80,padded_columns:96,stage_us:[1,2,3]}' > "$work/requests"
rejected 'a request without seven stage times'
jq -nc 'range(31)|{elapsed_us:100,valid_columns:80,padded_columns:96,stage_us:[1,2,3,4,5,6,7]}' > "$work/requests"
rejected 'incomplete request cohort'
# Distinct, unsorted latencies and stage times pin the nearest-rank indices.
for shape in '32 1016 1031 1032 15' '64 1032 1061 1064 31'; do
 read -r n p50 p95 p99 stage <<< "$shape"
 jq -nc --argjson n "$n" 'range($n) as $i|($i*13%$n) as $k|
  {elapsed_us:(1001+$k),valid_columns:80,padded_columns:96,stage_us:[range(7) as $s|1000*($s+1)+$k]}' > "$work/requests"
 run "$n" > "$work/result"
 jq -e --argjson p50 "$p50" --argjson p95 "$p95" --argjson p99 "$p99" --argjson stage "$stage" \
  '.p50_us==$p50 and .p95_us==$p95 and .p99_us==$p99 and .stage_p50_us==[range(7) as $s|1000*($s+1)+$stage]' \
  "$work/result" > /dev/null || { echo "nearest-rank quantiles of $n requests differ" >&2; exit 1; }
done
printf '%s\n' 'Ablation raw resource counter contracts passed.'
