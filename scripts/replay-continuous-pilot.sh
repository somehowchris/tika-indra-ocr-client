#!/usr/bin/env bash
# Offline analysis only. Does not execute OCI images, OCR, models or network calls.
set -euo pipefail
export LC_ALL=C
umask 077
[[ $# = 2 ]] || { echo "usage: $0 PACKET_DIRECTORY NEW_OUTPUT" >&2; exit 2; }
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
packet=$(realpath -e -- "$1")
output=$(realpath -m -- "$2")
[[ ! -e $output && ! -L $output ]]
sha() { sha256sum -- "$1" | awk '{print $1}'; }
(cd "$packet" && sha256sum --check --strict SHA256SUMS) >/dev/null
jq -e '.schema_version==1 and .admitted_cells==240 and .successful_requests==2880 and .paired_repetitions==120 and .engineering_screen_passed==false and .default_promotion==false' "$packet/receipt.json" >/dev/null
[[ $(sha "$packet/raw.tar.gz") = $(jq -er '.raw_archive_sha256' "$packet/receipt.json") ]]
[[ $(sha "$root/scripts/admit-continuous-pilot.py") = $(jq -er '.analyzer_sha256' "$packet/receipt.json") ]]
mkdir -p -- "$(dirname "$output")"
mkdir -m0700 -- "$output"
mkdir "$output/raw"
tar --extract --gzip --file "$packet/raw.tar.gz" --no-same-owner --no-same-permissions --directory "$output/raw"
python3 "$root/scripts/admit-continuous-pilot.py" \
 "$output/raw/continuous-native-85fef71-20261019" \
 "$output/raw/continuous-tika-85fef71-20261019" "$output/analysis"
cmp "$packet/paired.json" "$output/analysis/paired.json"
printf '%s\n' 'Offline replay passed: complete raw manifests, 240 cells, 120 pairs and the unchanged non-promotion result.'
