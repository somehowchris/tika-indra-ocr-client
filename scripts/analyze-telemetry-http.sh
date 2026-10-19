#!/usr/bin/env bash
# Descriptive paired cells, never a request-level population or tail inference.
set -euo pipefail
[[ $# = 1 ]]
study=$(realpath -e -- "$1")
jq -e '.mode=="run" and .status=="completed" and (.cells|length)==40 and all(.cells[];.status=="completed")' "$study/study.json" > /dev/null
[[ ! -e $study/analysis && ! -L $study/analysis ]]
mkdir "$study/analysis"
counter() { awk -v key="$2" '$1==key{if(found++)exit 1; value=$2}END{if(found!=1 || value !~ /^[0-9]+$/)exit 1; print value}' "$1"; }
for cpus in 1 2; do
 for repetition in {0..9}; do
  for arm in off enabled; do
   cell="$study/cpu-$cpus-r$repetition-$arm/run"
   jq -e --arg arm "$arm" --argjson cpus "$cpus" '.complete==true and .arm==$arm and .cpus==$cpus and .successful_requests==12 and .median_us>0 and .wall_us>0' "$cell/summary.json" > /dev/null
   jq -n --slurpfile summary "$cell/summary.json" \
    --argjson before "$(counter "$cell/cohort-before/cpu.stat" usage_usec)" \
    --argjson after "$(counter "$cell/cohort-after/cpu.stat" usage_usec)" \
    --argjson throttle_before "$(counter "$cell/cohort-before/cpu.stat" throttled_usec)" \
    --argjson throttle_after "$(counter "$cell/cohort-after/cpu.stat" throttled_usec)" \
    --argjson startup "$(counter "$cell/startup/cpu.stat" usage_usec)" \
    --argjson terminal "$(counter "$cell/terminal-after/cpu.stat" usage_usec)" \
    --argjson peak "$(cat "$cell/terminal-after/memory.peak")" \
    '$summary[0] as $s|if $after<$before or $terminal<$startup or $throttle_after<$throttle_before then error("counter decreased") else {median_us:$s.median_us,wall_us:$s.wall_us,cohort_cpu_us:($after-$before),cohort_throttled_us:($throttle_after-$throttle_before),startup_through_export_audit_cpu_us:($terminal-$startup),full_cell_memory_peak_bytes:$peak,cohort_exports_observed:$s.cohort_exports_observed} end' > "$study/analysis/cpu-$cpus-r$repetition-$arm.json"
  done
  jq -n --slurpfile off "$study/analysis/cpu-$cpus-r$repetition-off.json" --slurpfile enabled "$study/analysis/cpu-$cpus-r$repetition-enabled.json" --argjson cpus "$cpus" --argjson repetition "$repetition" '
   {cpus:$cpus,repetition:$repetition,order:(if $repetition%2==0 then ["off","enabled"] else ["enabled","off"] end),off:$off[0],enabled:$enabled[0],paired:["median_us","wall_us","cohort_cpu_us","cohort_throttled_us","startup_through_export_audit_cpu_us","full_cell_memory_peak_bytes"]|map(. as $metric|{metric:$metric,enabled_minus_off:($enabled[0][$metric]-$off[0][$metric]),enabled_divided_by_off:(if $off[0][$metric]>0 then $enabled[0][$metric]/$off[0][$metric] else null end)})}' > "$study/analysis/pair-cpu-$cpus-r$repetition.json"
 done
done
jq -s '{scope:"20 paired cohort observations; ten pairs per CPU budget; descriptive only, no independent request CI or p99 estimate",pairs:.}' "$study"/analysis/pair-*.json > "$study/analysis/paired.json"
