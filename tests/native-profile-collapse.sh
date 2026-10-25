#!/usr/bin/env bash
# External-tool format regression. Synthetic stacks; not OCR measurements.
set -euo pipefail
[[ $# = 3 && -x $1 && -x $2 && -f $3 ]] || { printf 'usage: %s INFERNO_COLLAPSE_PERF RUSTFILT PROFILE_NATIVE_SH\n' "$0" >&2; exit 1; }
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
# Real Rust v0 symbols of run 39839055833 (cpu-1/math): an ORT session call
# whose demangled name holds an array type, called from Engine::recognize.
for ((index=0; index<3; index++)); do
    printf 'ocr-profiling 42/42 123.000005: cpu-clock:u: \n\t0005 %s (/ocr-profiling)\n\t0006 %s (/ocr-profiling)\n\n' \
        _RINvMs4_NtCseEdmphlMmMF_3ort7sessionNtB6_7Session3runKj7_ANtNtB6_5input17SessionInputValueBQ_ECsjXFQbJxCqLY_12indra_engine \
        _RNvMs0_NtCsjXFQbJxCqLY_12indra_engine7runtimeNtB5_6Engine9recognize
done >> "$owned/perf-script.txt"
# Inferno parses the event name before its first colon, excluding the :u modifier.
"$1" --addrs --nthreads 1 --event-filter cpu-clock "$owned/perf-script.txt" > "$owned/folded.txt"
awk 'NF < 2 || $NF !~ /^[0-9]+$/ {exit 1} {total += $NF} END {if (total != 106) exit 1}' "$owned/folded.txt"
grep -Fq 'MlasConvPointwiseFloatKernelFma3' "$owned/folded.txt"
grep -Fq 'indra_engine::runtime::Engine::recognize' "$owned/folded.txt"
grep -Fq 'unknown' "$owned/folded.txt"
grep -Fq '(anonymous namespace)::MlasGeluErfKernelAvx512FExactImpl' "$owned/folded.txt"
grep -Fq '(anonymous namespace)::MlasErfKernelAvx512FImpl' "$owned/folded.txt"
if grep -Fq '(process marker)' "$owned/folded.txt" || grep -Fq ';(anonymous namespace) ' "$owned/folded.txt"; then exit 1; fi
# The demangled copy keeps every stack, frame and weight; only the two Rust
# symbols change, and the array type's ';' cannot split its frame.
RUSTFILT=$2 bash "$3" --demangle-folded "$owned/folded.txt" > "$owned/folded-demangled.txt"
cmp <(awk '{print $NF}' "$owned/folded.txt") <(awk '{print $NF}' "$owned/folded-demangled.txt")
grep -Fxq 'ocr-profiling;<indra_engine::runtime::Engine>::recognize;<ort::session::Session>::run::<7, [ort::session::input::SessionInputValue: 7]> 3' \
    "$owned/folded-demangled.txt"
if grep -Eq '_R[0-9]*[CMXYNIB][0-9A-Za-z_]' "$owned/folded-demangled.txt"; then exit 1; fi
grep -Fq '_RNvMs0_NtCsjXFQbJxCqLY_12indra_engine7runtimeNtB5_6Engine9recognize' "$owned/folded.txt"
cmp <(grep -v '_R' "$owned/folded.txt") <(grep -v '<indra_engine::runtime::Engine>' "$owned/folded-demangled.txt")
printf '{"scope":"synthetic external Inferno event/symbol and rustfilt v0 regression; empty and marker stacks remain omitted","headers":109,"folded_samples":106,"native_profile_admission":false,"passed":true}\n'
