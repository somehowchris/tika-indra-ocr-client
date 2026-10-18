#!/usr/bin/env bash
# Create one unique Docker/systemd parent without host sudo or broad cgroup access.
set -Eeuo pipefail
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; return 1; }
[[ $# == 6 ]] || { fail 'Usage: docker-evaluation-parent.sh create SLICE CPUS AFFINITY HELPER_IMAGE NEW_RECEIPT'; exit 2; }
[[ $1 == create ]] || exit 2
slice=$2 cpus=$3 affinity=$4 image=$5 receipt=$6
[[ $slice =~ ^indraeval[0-9]+n[0-9]+[a-f0-9]{16}\.slice$ && $cpus =~ ^[12]$ && $affinity =~ ^[0-9]+(,[0-9]+)?$ &&
   $image =~ ^sha256:[a-f0-9]{64}$ && ! -e $receipt && ! -L $receipt ]] || exit 2
group=/sys/fs/cgroup/$slice
[[ ! -e $group ]] || { fail 'Refusing existing parent'; exit 1; }
name=${slice%.slice}-bootstrap
token=$slice
cleanup() {
  local saved=$? identity
  trap - EXIT
  identity=$(docker inspect --format '{{index .Config.Labels "org.tika-indra.evaluation-parent"}}' "$name" 2>/dev/null || true)
  if [[ $identity == "$token" ]]; then docker rm -f "$name" >/dev/null || saved=1; fi
  exit "$saved"
}
trap cleanup EXIT
# Bootstrap is explicitly setup activity, never an OCR timing sample.
docker run -d --name "$name" --pull never --label "org.tika-indra.evaluation-parent=$token" \
  --cgroup-parent "$slice" --network none --read-only --cap-drop ALL --security-opt no-new-privileges \
  --cpus "$cpus" --cpuset-cpus "$affinity" --memory 32m --memory-swap 32m --pids-limit 8 \
  --entrypoint /bin/sleep "$image" infinity >/dev/null
actual=$(docker inspect --format '{{.Image}}' "$name")
[[ $actual == "$image" && -d $group && ! -L $group ]]
device=$(stat -c %d "$group"); inode=$(stat -c %i "$group")
docker inspect "$name" > "$receipt.bootstrap.json"
sentinel_id=$(docker inspect --format '{{.Id}}' "$name")
sentinel_pid=$(docker inspect --format '{{.State.Pid}}' "$name")
sentinel_cgroup=$(awk -F: '$1=="0"{print $3}' "/proc/$sentinel_pid/cgroup")
sentinel_start=$(sed 's/.*) //' "/proc/$sentinel_pid/stat" | awk '{print $20}')
[[ $sentinel_cgroup == /"$slice"/docker-"$sentinel_id".scope ]]
grep -Fxq 'populated 1' "$group/cgroup.events"
grep -Fxq 'nr_descendants 1' "$group/cgroup.stat"
# Keep the owned sleep process alive. An empty slice is deactivated by systemd;
# reactivation on the next Docker start would reset direct cgroup limits.
# The helper lives outside the measured parent and can access ONLY this exact
# owned directory. No privileged mode, host PID namespace, socket or root mount.
docker run --rm --pull never --network none --read-only --cap-drop ALL --security-opt no-new-privileges \
  --cpus 1 --memory 32m --memory-swap 32m --pids-limit 8 \
  --mount "type=bind,src=$group,dst=/owned" --entrypoint /bin/sh "$image" -ec '
    test "$(stat -c %d /owned)" = "$1"; test "$(stat -c %i /owned)" = "$2"
    grep -Fxq "populated 1" /owned/cgroup.events
    grep -Fxq "nr_descendants 1" /owned/cgroup.stat
    printf "%s 100000\n" "$3" > /owned/cpu.max
    printf "%s\n" "$4" > /owned/cpuset.cpus
    printf "3221225472\n" > /owned/memory.max
    printf "0\n" > /owned/memory.swap.max
    printf "384\n" > /owned/pids.max
  ' _ "$device" "$inode" "$((cpus*100000))" "$affinity"
[[ $(cat "$group/cpu.max") == "$((cpus*100000)) 100000" && $(cat "$group/memory.max") == 3221225472 &&
   $(cat "$group/memory.swap.max") == 0 && ( $(cat "$group/cpuset.cpus.effective") == "$affinity" || $(cat "$group/cpuset.cpus.effective") == "${affinity/,/-}" ) ]]
jq -n --arg slice "$slice" --arg group "$group" --arg image "$image" --argjson device "$device" --argjson inode "$inode" \
  --arg sentinel_id "$sentinel_id" --arg sentinel_name "$name" --argjson sentinel_pid "$sentinel_pid" \
  --arg sentinel_cgroup "$sentinel_cgroup" --argjson sentinel_start "$sentinel_start" \
  --arg cpu_stat "$(cat "$group/cpu.stat")" --argjson cpu_usage "$(awk '$1=="usage_usec"{print $2}' "$group/cpu.stat")" --argjson memory_peak "$(cat "$group/memory.peak")" \
  '{schema_version:1,slice:$slice,path:$group,helper_image_id:$image,device:$device,inode:$inode,sentinel_id:$sentinel_id,sentinel_name:$sentinel_name,sentinel_pid:$sentinel_pid,sentinel_cgroup:$sentinel_cgroup,sentinel_start_ticks:$sentinel_start,setup_cpu_stat:$cpu_stat,setup_cpu_usage_usec:$cpu_usage,setup_memory_peak_bytes:$memory_peak,
    accounting:"Bootstrap setup CPU remains in the retained baseline; measured CPU uses before/after deltas. The owned sleeping sentinel remains in the aggregate budget. Parent lifetime memory peak includes sentinel, setup and workload startup. No memory peak subtraction is valid.",
    cleanup:"After measured workloads are removed, stop/remove only the receipt-bound sentinel; retain the resulting empty slice without host service administration."}' > "$receipt"
trap - EXIT
