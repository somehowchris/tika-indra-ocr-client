#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# The caller builds trace-acceptance from immutable production image identities.
set -euo pipefail
if [ "$#" -ne 3 ]; then
  printf 'usage: %s TRACE_ACCEPTANCE_IMAGE VERIFIED_MODEL_DIRECTORY NEW_EVIDENCE_DIRECTORY\n' "$0" >&2
  exit 2
fi
image=$1
models=$(realpath "$2")
evidence=$3
[[ -d "$models" && "$evidence" = /* && ! -e "$evidence" ]]
mkdir -p "$evidence"
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
bash "$root/scripts/verify-daemon-image.sh" resolve "$image" "$evidence/image-identity"
docker image inspect "$image" > "$evidence/image.json"
lscpu --json > "$evidence/lscpu.json"
failed=0
owned_cid_file=
cleanup() {
  local status=$? id
  trap - EXIT
  if [[ -n "$owned_cid_file" && -f "$owned_cid_file" ]]; then
    id=$(cat "$owned_cid_file")
    if [[ "$id" =~ ^[0-9a-f]{64}$ ]]; then
      timeout --signal=TERM --kill-after=5s 20s docker kill "$id" >/dev/null 2>&1 || true
      timeout --signal=TERM --kill-after=5s 20s docker inspect "$id" > "${owned_cid_file%.id}-aborted.json" 2>/dev/null || true
      timeout --signal=TERM --kill-after=5s 20s docker rm --force "$id" >/dev/null || status=1
    else status=1; fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for cpus in 1 2; do
  affinity=${TRACE_CPUSET_1:-0}
  if [ "$cpus" = 2 ]; then affinity=${TRACE_CPUSET_2:-0,1}; fi
  directory="$evidence/$cpus-cpu"
  mkdir "$directory"
  name="indra-trace-acceptance-$cpus-$$-$RANDOM"
  owned_cid_file="$directory/container.id"
  if ! timeout --signal=TERM --kill-after=15s 900s docker run --cidfile "$owned_cid_file" --name "$name" --network=none --read-only \
    --user "$(id -u):$(id -g)" --cpus="$cpus" --cpuset-cpus="$affinity" \
    --memory=3g --memory-swap=3g --pids-limit=256 --cap-drop=ALL \
    --security-opt=no-new-privileges --tmpfs /tmp:rw,nosuid,nodev,size=64m,mode=1777 \
    --mount "type=bind,src=$models,dst=/models,readonly" \
    --mount "type=bind,src=$directory,dst=/evidence" \
    "$image" --gateway /indra-gateway --java /usr/bin/java \
    --jar /opt/tika/tika-server.jar --bridge /opt/indra/tesseract \
    --native /indra-server --models /models --evidence /evidence/run --threads "$cpus" \
    > "$directory/console.log" 2>&1; then failed=1; fi
  id=$(cat "$owned_cid_file")
  [[ "$id" =~ ^[0-9a-f]{64}$ ]]
  docker inspect "$id" > "$directory/container.json"
  if ! bash "$root/scripts/verify-daemon-image.sh" bind "$evidence/image-identity" \
    "$directory/container.json" "$directory/executed-identity"; then failed=1; fi
  if ! jq --exit-status --argjson cpu "$((cpus * 1000000000))" --arg affinity "$affinity" '
    length == 1 and .[0].HostConfig.NanoCpus == $cpu and
    .[0].HostConfig.CpusetCpus == $affinity and
    .[0].HostConfig.Memory == 3221225472 and .[0].HostConfig.MemorySwap == 3221225472 and
    .[0].HostConfig.NetworkMode == "none" and .[0].HostConfig.ReadonlyRootfs == true and
    .[0].State.OOMKilled == false and .[0].State.ExitCode == 0
  ' "$directory/container.json" > "$directory/limits-and-exit-verified.txt"; then failed=1; fi
  docker rm "$id" > "$directory/removed.txt"
  owned_cid_file=
done
printf 'scope=correctness, privacy, graph identity and shutdown flushing; not a performance result\n' > "$evidence/scope.txt"
exit "$failed"
