#!/usr/bin/env bash
# Real Docker infrastructure acceptance. No OCR or performance claims.
set -Eeuo pipefail
[[ $# == 4 ]] || { echo 'Usage: test-docker-evaluation-parent.sh IMAGE_ID NEW_OUTPUT CPU_ONE CPU_TWO' >&2; exit 2; }
image=$1 output=$2 one=$3 two=$4
[[ $image =~ ^sha256:[a-f0-9]{64}$ && ! -e $output && $one =~ ^[0-9]+$ && $two =~ ^[0-9]+,[0-9]+$ ]] || exit 2
script_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
# shellcheck source=evaluate-ocr-matrix.sh
source "$script_root/evaluate-ocr-matrix.sh"
mkdir -p "$output/control"
MATRIX_RESOURCE_BACKEND=docker MATRIX_HELPER_IMAGE=$image
owned_children=()
active_receipt='' active_slice=''
cleanup() {
  local saved=$?
  trap - EXIT
  for child in "${owned_children[@]}"; do
    local owner
    owner=$(docker inspect --format '{{index .Config.Labels "org.tika-indra.parent-test"}}' "$child" 2>/dev/null || true)
    [[ $owner != "$active_slice" ]] || docker rm -f "$child" >/dev/null || saved=1
  done
  if [[ -n $active_receipt && -f $active_receipt ]]; then
    release_slice "$output" "$active_slice" "$active_receipt" || saved=1
  fi
  exit "$saved"
}
trap cleanup EXIT
for cpus in 1 2; do
  affinity=$one; [[ $cpus == 1 ]] || affinity=$two
  nonce=$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')
  active_slice=indraeval0n${cpus}${nonce}.slice
  active_receipt=$output/control/$active_slice
  capture "$output" "parent-$cpus-create" 180 bash "$script_root/docker-evaluation-parent.sh" \
    create "$active_slice" "$cpus" "$affinity" "$image" "$active_receipt"
  group=/sys/fs/cgroup/$active_slice
  for position in 1 2 3; do
    child=indra-parent-test-${nonce}-$position
    owned_children+=("$child")
    capture "$output" "parent-$cpus-child-$position" 30 docker run -d --name "$child" --pull never \
      --label "org.tika-indra.parent-test=$active_slice" --cgroup-parent "$active_slice" \
      --network none --read-only --cap-drop ALL --security-opt no-new-privileges \
      --cpus "$cpus" --cpuset-cpus "$affinity" --memory 32m --memory-swap 32m --pids-limit 8 \
      --entrypoint /bin/sleep "$image" infinity
    capture "$output" "parent-$cpus-child-$position-inspect" 30 docker inspect "$child"
    [[ $(cat "$group/cpu.max") == "$((cpus*100000)) 100000" && $(cat "$group/memory.max") == 3221225472 && $(cat "$group/memory.swap.max") == 0 ]]
    for counter in cpu.max cpu.stat cpuset.cpus.effective memory.max memory.swap.max memory.current memory.peak cgroup.stat cgroup.events; do
      cp "$group/$counter" "$output/control/parent-$cpus-after-$position-$counter"
    done
  done
  for child in "${owned_children[@]}"; do docker rm -f "$child" >/dev/null; done
  owned_children=()
  release_slice "$output" "$active_slice" "$active_receipt"
  active_receipt='' active_slice=''
done
printf '%s\n' 'Passed: aggregate 1/2 CPU, 3 GiB, zero swap survive three sibling starts; owned sentinels and children removed. No OCR or timing measurements.' > "$output/result.txt"
trap - EXIT
