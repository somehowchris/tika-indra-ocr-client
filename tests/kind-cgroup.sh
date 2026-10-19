#!/usr/bin/env bash
# Synthetic host cgroup records reproduce the ownership/limit distinction. No
# Docker daemon, root access, cgroup writes or workload execution is involved.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck source=../scripts/lib/kind-cgroup.sh
source "$root/scripts/lib/kind-cgroup.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
id=37628595447a4af1974e2d80c80676abf2769dbe24d27c88bad24b90d5478acd
other=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
owner=/system.slice/docker-$id.scope
for suffix in '' /init.scope /system.slice/containerd.service; do
    printf '0::%s%s\n' "$owner" "$suffix" > "$work/process.cgroup"
    [[ $(kind_cgroup_owner "$id" "$work/process.cgroup") = "$owner" ]]
done
reject() {
    printf '%b' "$1" > "$work/process.cgroup"
    if kind_cgroup_owner "$id" "$work/process.cgroup" > "$work/rejected.stdout"; then
        echo "incorrectly accepted cgroup record: $2" >&2; exit 1
    fi
    [[ ! -s "$work/rejected.stdout" ]]
}
reject "0::/system.slice/docker-$other.scope/init.scope\n" wrong-owner
reject "0::/system.slice/not-docker-$id.scope/init.scope\n" substring
reject "0::/system.slice/$id/init.scope\n" wrong-driver
reject "0::/system.slice/docker-$id.scope-extra/init.scope\n" prefix
reject "0::$owner/docker-$id.scope/init.scope\n" repeated-owner
reject "0::$owner/docker-$other.scope/init.scope\n" foreign-nested-owner
reject "0::$owner/../init.scope\n" traversal
reject "0::$owner//init.scope\n" empty-component
reject "0::$owner/./init.scope\n" dot-component
reject "0::$owner/init.scope/\n" trailing-slash
reject "0::$owner/init.scope\n0::$owner/init.scope\n" duplicate-lines
reject "1:cpu:$owner/init.scope\n" v1-controller
reject "0::$owner/init.scope" incomplete-record
reject "0::$owner/init.scope\r\n" carriage-return
reject "0::$owner/init\0.scope\n" embedded-NUL
printf '0::%s' "$owner" > "$work/process.cgroup"
for ((depth=0; depth<64; depth++)); do printf /scope >> "$work/process.cgroup"; done
printf '\n' >> "$work/process.cgroup"
if kind_cgroup_owner "$id" "$work/process.cgroup"; then echo 'accepted excessive depth' >&2; exit 1; fi
printf '0::%s/init.scope\n' "$owner" > "$work/original"
ln -s "$work/original" "$work/symlink"
if kind_cgroup_owner "$id" "$work/symlink"; then echo 'accepted symlink record' >&2; exit 1; fi
# These are the exact quota bytes observed in the failed CI replay, applied to
# a synthetic PID leaf. `max` must still fail if it appears at the owned scope.
printf 'max 100000\n' > "$work/leaf-cpu.max"
mkdir -p "$work/cgroups$owner/init.scope"
printf '0::%s/init.scope\n' "$owner" > "$work/nested-process.cgroup"
cp "$work/leaf-cpu.max" "$work/cgroups$owner/init.scope/cpu.max"
printf '100000 100000\n' > "$work/cgroups$owner/cpu.max"
old_path=$(cat "$work/nested-process.cgroup");old_path=${old_path#0::}
if kind_cgroup_quota "$work/cgroups$old_path/cpu.max" 1; then echo 'old PID leaf unexpectedly passed' >&2; exit 1; fi
selected_owner=$(kind_cgroup_owner "$id" "$work/nested-process.cgroup")
kind_cgroup_quota "$work/cgroups$selected_owner/cpu.max" 1
printf 'max 100000\n' > "$work/cgroups$selected_owner/cpu.max"
if kind_cgroup_quota "$work/cgroups$selected_owner/cpu.max" 1; then echo 'new owner accepted unlimited quota' >&2; exit 1; fi
for cpus in 1 2; do
    printf '%s 100000\n' "$((cpus*100000))" > "$work/owner-cpu.max"
    kind_cgroup_quota "$work/owner-cpu.max" "$cpus"
    if kind_cgroup_quota "$work/leaf-cpu.max" "$cpus"; then echo 'accepted unlimited owner quota' >&2; exit 1; fi
    if kind_cgroup_quota "$work/owner-cpu.max" "$((3-cpus))"; then echo 'accepted wrong CPU quota' >&2; exit 1; fi
done
for quota in '100000 0' '100000 100000 extra' 'max 100000' '100000 100000\n200000 100000' '99999 100000' '999999999999999999999999 100000'; do
    printf '%b\n' "$quota" > "$work/invalid-cpu.max"
    if kind_cgroup_quota "$work/invalid-cpu.max" 1; then echo "accepted malformed/wrong owner quota: $quota" >&2; exit 1; fi
done
printf '%s\n' 'Kind cgroup fixtures passed: exact Docker scope, nested PID leaves, owner/driver/traversal negatives and strict numeric 1/2 CPU quotas.'
