#!/usr/bin/env bash
# Read-only statistical/wire replay. No model, OCI daemon or network access.
set -euo pipefail
export LC_ALL=C
umask 077
[[ $# = 3 ]] || { echo "usage: $0 STATIC_VERIFIER PACKET_DIRECTORY NEW_OUTPUT" >&2; exit 2; }
binary=$(realpath -e -- "$1")
packet=$(realpath -e -- "$2")
output=$(realpath -m -- "$3")
[[ -x $binary && ! -e $output && ! -L $output ]]
sha() { sha256sum -- "$1" | awk '{print $1}'; }
(cd "$packet" && sha256sum --check --strict SHA256SUMS) >/dev/null
[[ $(sha "$binary") = $(jq -er '.verifier_binary_sha256' "$packet/offline-verifier.json") ]]
[[ $(sha "$packet/raw-complete.tar.gz") = $(jq -er '.raw_archive_sha256' "$packet/offline-verifier.json") ]]
mkdir -p -- "$(dirname "$output")"
mkdir -m0700 -- "$output"
mkdir "$output/raw" "$output/admission"
tar --extract --gzip --file "$packet/raw-complete.tar.gz" --no-same-owner --no-same-permissions --directory "$output/raw"
for name in telemetry-http-correctness-d4d80fe-http28-20261019 telemetry-http-correctness-646a912-http28-20261019 telemetry-http-correctness-bd1a6d0-http28-20261019; do
 (cd "$output/raw/$name" && sha256sum --check --strict SHA256SUMS) >/dev/null
done
study="$output/raw/telemetry-http-correctness-d4d80fe-http28-20261019"
jq -e '.exit==0 and .cleanup_exit==0' "$study/status.json" >/dev/null
jq -e '.mode=="correctness" and .status=="completed" and (.cells|length)==4 and (.references|length)==2 and all(.cells[];.status=="completed") and all(.references[];.status=="completed")' "$study/study.json" >/dev/null
for cpus in 1 2; do
 for arm in reference off enabled; do
  cell="cpu-$cpus-r0-$arm"
  [[ $arm != reference ]] || cell="reference-cpu-$cpus"
  before="$study/$cell/container-before.json"
  after="$study/$cell/container-after.json"
  jq -e --slurpfile before "$before" 'length==1 and ($before|length)==1 and ($before[0]|length)==1 and (.[0]|.Id==$before[0][0].Id and .Image==$before[0][0].Image and .Config.Labels["org.tika-indra.telemetry-study"]==$before[0][0].Config.Labels["org.tika-indra.telemetry-study"] and .State.Running==false and .State.ExitCode==0 and .State.OOMKilled==false)' "$after" >/dev/null
  [[ $(cat "$study/$cell/removed.txt") = $(jq -er '.[0].Id' "$after") ]]
  arguments=()
  [[ $arm = reference ]] || arguments=(--goldens "$study/reference-cpu-$cpus/run")
  "$binary" verify --evidence "$study/$cell/run" --identity "$study/control/build-identity.json" "${arguments[@]}" --output "$output/admission/$cell.json"
 done
done
jq -s 'if length!=6 or any(.[];.accepted!=true) then error("incomplete offline admission") else {scope:"Offline replay of retained mixed-build correctness only; no inference or measured overhead",cells:.,admitted_cells:length,successful_requests:(map(.successful_requests)|add)} end' "$output"/admission/*.json > "$output/summary.json"
printf '%s\n' 'Offline telemetry replay passed: all original raw checksums, six actual wire/privacy/graph/role/outcome admissions and 48 exact response hashes.'
