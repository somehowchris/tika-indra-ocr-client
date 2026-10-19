#!/usr/bin/env bash
# External-tool format regression. Synthetic stacks; not OCR measurements.
set -euo pipefail
[[ $# = 1 && -x $1 ]] || { printf 'usage: %s INFERNO_COLLAPSE_PERF\n' "$0" >&2; exit 1; }
owned=$(mktemp -d "${TMPDIR:-/tmp}/indra-collapse-XXXXXXXX")
trap 'rm -rf -- "$owned"' EXIT
for ((index=0; index<100; index++)); do
    printf 'ocr-profiling 42/42 123.000001: cpu-clock:u: \n\t0001 MlasConvPointwiseFloatKernelFma3 (/ocr-profiling)\n\t0002 indra_engine::runtime::Engine::recognize (/ocr-profiling)\n\n'
done > "$owned/perf-script.txt"
# Real symbols observed in failed run39233052354; these two single-frame samples
# must survive. Marker names and a header-only sample must keep being omitted.
for symbol in \
    '(anonymous namespace)::MlasGeluErfKernelAvx512FExactImpl(float const*, float*, unsigned long)' \
    '(anonymous namespace)::MlasErfKernelAvx512FImpl(float const*, float*, unsigned long)' \
    '(process marker)' '(anonymous namespace)'; do
    printf 'ocr-profiling 42/42 123.000002: cpu-clock:u: \n\t0003 %s (/ocr-profiling)\n\n' "$symbol"
done >> "$owned/perf-script.txt"
printf 'ocr-profiling 42/42 123.000004: cpu-clock:u: \n\t0004 [unknown] ([unknown])\n\n' >> "$owned/perf-script.txt"
printf 'ocr-profiling 42/42 123.000003: cpu-clock:u: \n\n' >> "$owned/perf-script.txt"
# Inferno parses the event name before its first colon, excluding the :u modifier.
"$1" --addrs --nthreads 1 --event-filter cpu-clock "$owned/perf-script.txt" > "$owned/folded.txt"
awk 'NF < 2 || $NF !~ /^[0-9]+$/ {exit 1} {total += $NF} END {if (total != 103) exit 1}' "$owned/folded.txt"
grep -Fq 'MlasConvPointwiseFloatKernelFma3' "$owned/folded.txt"
grep -Fq 'indra_engine::runtime::Engine::recognize' "$owned/folded.txt"
grep -Fq 'unknown' "$owned/folded.txt"
grep -Fq '(anonymous namespace)::MlasGeluErfKernelAvx512FExactImpl' "$owned/folded.txt"
grep -Fq '(anonymous namespace)::MlasErfKernelAvx512FImpl' "$owned/folded.txt"
if grep -Fq '(process marker)' "$owned/folded.txt" || grep -Fq ';(anonymous namespace) ' "$owned/folded.txt"; then exit 1; fi
printf '{"scope":"synthetic external Inferno event/symbol regression; empty and marker stacks remain omitted","headers":106,"folded_samples":103,"native_profile_admission":false,"passed":true}\n'
