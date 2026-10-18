#!/usr/bin/env bash
# External-tool format regression. Synthetic stacks; not OCR measurements.
set -euo pipefail
[[ $# = 1 && -x $1 ]] || { printf 'usage: %s INFERNO_COLLAPSE_PERF\n' "$0" >&2; exit 1; }
owned=$(mktemp -d "${TMPDIR:-/tmp}/indra-collapse-XXXXXXXX")
trap 'rm -rf -- "$owned"' EXIT
for ((index=0; index<100; index++)); do
    printf 'ocr-profiling 42/42 123.000001: cpu-clock:u: \n\t0001 MlasConvPointwiseFloatKernelFma3 (/ocr-profiling)\n\t0002 indra_engine::runtime::Engine::recognize (/ocr-profiling)\n\n'
done > "$owned/perf-script.txt"
# Inferno parses the event name before its first colon, excluding the :u modifier.
"$1" --addrs --nthreads 1 --event-filter cpu-clock "$owned/perf-script.txt" > "$owned/folded.txt"
awk 'NF < 2 || $NF !~ /^[0-9]+$/ {exit 1} {total += $NF} END {if (total != 100) exit 1}' "$owned/folded.txt"
grep -Fq 'MlasConvPointwiseFloatKernelFma3' "$owned/folded.txt"
grep -Fq 'indra_engine::runtime::Engine::recognize' "$owned/folded.txt"
printf '{"scope":"synthetic external Inferno event-modifier regression","samples":100,"passed":true}\n'
