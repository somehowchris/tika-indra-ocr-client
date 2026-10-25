#!/usr/bin/env bash
# Retained evidence packets are hash-bound records: every SHA256SUMS in the
# tree, every receipt.json file map and every synthetic-fixture case
# manifest must still match the files it names.
set -euo pipefail
export LC_ALL=C
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$repository"

receipts=0
while IFS= read -r receipt; do
  case $receipt in
    # A copy of the SDK producer container's /native/evidence listing, which
    # names files under absolute container paths; the packet's own receipt is
    # receipts.sha256.json, verified by evaluation/build-cost/test_sdk_evidence.py.
    evaluation/build-cost/evidence/native-sdk-2026-10-18/native-build-evidence/SHA256SUMS) continue ;;
  esac
  (cd -- "$(dirname -- "$receipt")" && sha256sum --check --strict --quiet SHA256SUMS) ||
    { printf '%s does not match its files\n' "$receipt" >&2; exit 1; }
  receipts=$((receipts + 1))
done < <(git ls-files -- '*SHA256SUMS')
[[ $receipts -gt 0 ]]

# receipt.json records that bind packet files: .files maps each path, relative
# to the receipt, to its sha256 and bytes.
maps=0
while IFS= read -r receipt; do
  case $receipt in
    # A byte copy of the hosted render's receipt, whose file map names the
    # render tree (evaluation/report/study-data/...); the packet's SHA256SUMS
    # and evaluation/report/test_frozen_study_data.py bind the committed copies.
    evaluation/results/frozen-study-*/render-receipt.json) continue ;;
  esac
  jq -e 'type == "object" and has("files")' "$receipt" > /dev/null || continue
  directory=$(dirname -- "$receipt")
  jq -r '.files | to_entries[] | "\(.value.sha256)  \(.key)"' "$receipt" |
    (cd -- "$directory" && sha256sum --check --strict --quiet -) ||
    { printf '%s does not match its files\n' "$receipt" >&2; exit 1; }
  while IFS=$'\t' read -r bytes file; do
    [[ $(wc -c < "$directory/$file") -eq $bytes ]] ||
      { printf '%s: %s is not %s bytes\n' "$receipt" "$file" "$bytes" >&2; exit 1; }
  done < <(jq -r '.files | to_entries[] | "\(.value.bytes)\t\(.key)"' "$receipt")
  maps=$((maps + 1))
done < <(git ls-files -- '*receipt.json')
[[ $maps -gt 0 ]]

fixtures=benchmarks/results/synthetic-fixtures
for manifest in "$fixtures"/*.json; do
  jq -r '.cases[] | "\(.input_sha256)  \(.input)", "\(.truth_sha256)  \(.truth)"' "$manifest" |
    (cd -- "$fixtures" && sha256sum --check --strict --quiet -) ||
    { printf '%s does not match its fixtures\n' "$manifest" >&2; exit 1; }
done
