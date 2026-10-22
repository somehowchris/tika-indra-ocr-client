#!/usr/bin/env bash
# Replay original sampling admission from immutable compact packet streams.
set -euo pipefail
[[ $# = 4 ]] || { echo 'usage: replay-compact-native-profile.sh PACKET FROZEN_SOURCE FULL_SOURCE_COMMIT NEW_OUTPUT' >&2; exit 2; }
packet=$(realpath -e -- "$1")
source=$(realpath -e -- "$2")
commit=$3
output=$(realpath -m -- "$4")
[[ $commit =~ ^[0-9a-f]{40}$ && ! -e $output && ! -L $output ]]
[[ $(git -C "$source" rev-parse HEAD) = "$commit" ]]
git -C "$source" diff --quiet HEAD --
(cd "$packet" && sha256sum --check --strict SHA256SUMS) >/dev/null
mkdir -m 0700 -- "$output"
for cpus in 1 2; do
    for fixture in tiff math; do
        raw="$packet/cpu-$cpus/$fixture"
        destination="$output/cpu-$cpus/$fixture"
        mkdir -p -- "$destination"
        expected=$(awk '$2=="/work/scripts/verify-native-profile-samples.sh" {print $1}' "$raw/execution-identities.sha256")
        [[ $expected =~ ^[0-9a-f]{64}$ ]]
        printf '%s  %s\n' "$expected" "$source/scripts/verify-native-profile-samples.sh" | sha256sum --check --status
        for name in perf-report perf-script; do
            timeout --signal=TERM --kill-after=5s 30s gzip --decompress --stdout "$raw/$name.txt.gz" > "$destination/$name.txt"
        done
        timeout --signal=TERM --kill-after=5s 30s /bin/bash "$source/scripts/verify-native-profile-samples.sh" \
            "$destination/perf-report.txt" "$destination/perf-script.txt" "$raw/recorded.jsonl" \
            "$raw/folded.txt" "$destination/sample-quality.json"
        cmp -- "$raw/sample-quality.json" "$destination/sample-quality.json"
    done
done
printf 'PASS: four original raw sample admissions replayed exactly from compact packet; original streams and figures unchanged.\n'
