#!/usr/bin/env bash
# Real Linux newline-terminated counters and invalid evidence, no OCR or compiler.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
jq -nc 'range(32)|{elapsed_us:100,valid_columns:80,padded_columns:96}' > "$work/requests"
printf 'usage_usec 10\nthrottled_usec 1\n' > "$work/before"
printf 'usage_usec 130\nthrottled_usec 5\n' > "$work/after"
run() {
 jq -n --argjson metadata '{"id":"real-counter-fixture"}' --slurpfile requests "$work/requests" \
  --rawfile before "$work/before" --rawfile after "$work/after" --rawfile peak "$work/peak" \
  --from-file "$root/scripts/native-ablation-resources.jq"
}
printf '131923968\n' > "$work/peak"
run > "$work/result"
jq -e '.request_count==32 and .phase_peak_memory_bytes==131923968 and .phase_cpu_usage_us==120 and .phase_throttled_us==4 and .p50_us==100' "$work/result" > /dev/null
for invalid in '' '0\n' '-1\n' 'NaN\n' '1\n2\n' '1\n\n' '3221225473\n'; do
 printf '%b' "$invalid" > "$work/peak"
 if run > "$work/bad.stdout" 2> "$work/bad.stderr"; then echo 'invalid peak admitted' >&2;exit 1;fi
done
printf '131923968\n' > "$work/peak"
printf 'usage_usec 130\nusage_usec 140\nthrottled_usec 5\n' > "$work/after"
if run > "$work/bad.stdout" 2> "$work/bad.stderr"; then echo 'duplicate CPU counter admitted' >&2;exit 1;fi
printf 'usage_usec 9\nthrottled_usec 5\n' > "$work/after"
if run > "$work/bad.stdout" 2> "$work/bad.stderr"; then echo 'decreasing CPU counter admitted' >&2;exit 1;fi
printf 'usage_usec 130\nthrottled_usec 5\n' > "$work/after"
jq -nc 'range(31)|{elapsed_us:100,valid_columns:80,padded_columns:96}' > "$work/requests"
if run > "$work/bad.stdout" 2> "$work/bad.stderr"; then echo 'incomplete request cohort admitted' >&2;exit 1;fi
printf '%s\n' 'Ablation raw resource counter contracts passed.'
