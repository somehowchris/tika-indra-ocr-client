#!/usr/bin/env bash
# Export one verified acceptance helper as a local BuildKit named context.
set -euo pipefail
export LC_ALL=C
umask 077
fail() { printf 'trace helper preparation: %s\n' "$*" >&2; exit 1; }
[[ $# = 2 ]] || fail 'usage: prepare-trace-helper.sh RESOLUTION_DIRECTORY NEW_EVIDENCE_DIRECTORY'
resolution=$(realpath -e -- "$1")
output=$(realpath -m -- "$2")
[[ -d $resolution && ! -e $output && ! -L $output ]] || fail 'verified resolution and new output required'
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p -- "$(dirname -- "$output")"
mkdir -m 0700 -- "$output"
mkdir "$output/context"
began=$SECONDS
bounded() {
    local limit=$1 remaining=$((1200 - (SECONDS - began)))
    shift
    (( remaining > 0 )) || fail 'overall preparation deadline exceeded'
    (( limit <= remaining )) || limit=$remaining
    timeout --signal=TERM --kill-after=5s "${limit}s" "$@" < /dev/null
}
capture() {
    local name=$1 seconds=$2 kib=$3
    shift 3
    local status=0
    (ulimit -f "$kib"; bounded "$seconds" "$@") > "$output/$name.stdout" 2> "$output/$name.stderr" || status=$?
    printf '%s\t%s\n' "$name" "$status" >> "$output/steps.tsv"
    (( status == 0 )) || fail "$name failed; retained command output has details"
}
jq -se 'length==1 and (.[0] | .passed==true and .store=="containerd" and
    .os=="linux" and .architecture=="amd64" and .image_id==.root_digest and
    (.root_digest|test("^sha256:[0-9a-f]{64}$"))) | select(.)' "$resolution/result.json" > /dev/null \
    || fail 'invalid acceptance root result'
image=$(jq -er .root_digest "$resolution/result.json")
label="indra-trace-helper-$$-$RANDOM"
container=
printf 'stage\texit_status\n' > "$output/steps.tsv"
finish() {
    local status=$? id
    trap - EXIT
    if [[ -f $output/container.id ]]; then
        id=$(cat "$output/container.id")
        if [[ "$id" =~ ^[0-9a-f]{64}$ ]]; then
            if timeout --signal=TERM --kill-after=5s 30s docker inspect "$id" > "$output/cleanup-inspect.json" 2> "$output/cleanup-inspect.stderr"; then
                if jq -e --arg id "$id" --arg owner "$label" --arg image "$image" '
                    length==1 and (.[0]|.Id==$id and .Image==$image and
                    .Config.Labels["org.tika-indra.trace-helper"]==$owner and .State.Running==false)
                ' "$output/cleanup-inspect.json" > /dev/null; then
                    timeout --signal=TERM --kill-after=5s 30s docker rm "$id" > "$output/removed.txt" 2> "$output/removed.stderr" || status=1
                else status=1; fi
            else status=1; fi
        else status=1; fi
    fi
    printf '%s\n' "$status" > "$output/exit-status.txt"
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
capture create 40 1024 docker create --platform linux/amd64 --network none --read-only \
    --cap-drop ALL --security-opt no-new-privileges:true \
    --label "org.tika-indra.trace-helper=$label" --cidfile "$output/container.id" "$image"
container=$(cat "$output/container.id")
[[ "$container" =~ ^[0-9a-f]{64}$ && $(cat "$output/create.stdout") = "$container" ]] || fail 'missing exact owned container ID'
capture inspect 30 1024 docker inspect "$container"
jq -e --arg id "$container" --arg owner "$label" '
    length==1 and (.[0]|.Id==$id and .State.Running==false and
    .Config.Labels["org.tika-indra.trace-helper"]==$owner and
    .HostConfig.NetworkMode=="none" and .HostConfig.ReadonlyRootfs==true)
' "$output/inspect.stdout" > /dev/null || fail 'created container ownership/isolation differs'
capture bind 950 1024 bash "$root/scripts/verify-daemon-image.sh" bind \
    "$resolution" "$output/inspect.stdout" "$output/binding"
jq -e --arg image "$image" --arg id "$container" '
    .passed==true and .root_digest==$image and .container_id==$id
' "$output/binding/result.json" > /dev/null || fail 'bound container root differs'
capture copy-helper 60 524288 docker cp "$container:/indra-trace-acceptance" "$output/context/indra-trace-acceptance"
capture copy-build-manifest 30 64 docker cp "$container:/build-evidence/binaries.sha256" "$output/build-binaries.sha256"
binary="$output/context/indra-trace-acceptance"
[[ -f $binary && ! -L $binary && -x $binary ]] || fail 'helper must be a regular executable'
bytes=$(stat -c %s -- "$binary")
(( bytes > 0 && bytes <= 536870912 )) || fail 'helper exceeds512MiB bound'
[[ -f $output/build-binaries.sha256 && ! -L $output/build-binaries.sha256 ]] || fail 'build manifest must be regular'
manifest_bytes=$(stat -c %s -- "$output/build-binaries.sha256")
(( manifest_bytes > 0 && manifest_bytes <= 65536 )) || fail 'build manifest exceeds64KiB'
expected=$(awk '$2=="/out/indra-trace-acceptance" {
    if(NF!=2 || length($1)!=64 || $1~/[^0-9a-f]/) bad=1
    n++; value=$1
} END {if(bad || n!=1) exit 1; print value}' "$output/build-binaries.sha256") || fail 'missing or duplicate helper build hash'
actual=$(sha256sum -- "$binary");actual=${actual%% *}
[[ "$actual" = "$expected" ]] || fail 'copied helper differs from bound build manifest'
capture elf-header 30 1024 readelf -W -h "$binary"
capture elf-program-headers 30 1024 readelf -W -l "$binary"
capture elf-dynamic 30 1024 readelf -W -d "$binary"
grep -Eq 'Class:[[:space:]]+ELF64' "$output/elf-header.stdout" || fail 'helper is not ELF64'
grep -Eq 'Machine:[[:space:]]+Advanced Micro Devices X86-64' "$output/elf-header.stdout" || fail 'helper is not x86_64'
if grep -q INTERP "$output/elf-program-headers.stdout" || grep -q NEEDED "$output/elf-dynamic.stdout"; then fail 'helper is not statically linked'; fi
chmod 0555 "$binary"
sha256sum -- "$root/scripts/prepare-trace-helper.sh" "$root/scripts/verify-daemon-image.sh" > "$output/controller-sources.sha256"
manifest_sha=$(sha256sum -- "$output/build-binaries.sha256");manifest_sha=${manifest_sha%% *}
jq -n --arg root "$image" --arg id "$container" --arg helper "$actual" --arg manifest "$manifest_sha" --argjson bytes "$bytes" \
    --slurpfile binding "$output/binding/result.json" '
    {schema_version:1,passed:true,root_digest:$root,platform_digest:$binding[0].platform_digest,
     config_digest:$binding[0].config_digest,container_id:$id,helper_sha256:$helper,
     helper_bytes:$bytes,build_manifest_sha256:$manifest,context_file:"context/indra-trace-acceptance",
     scope:"Unstarted bound acceptance container; exact static helper bytes exported for a local BuildKit named context; no registry publication or trace execution"}
' > "$output/result.json"
