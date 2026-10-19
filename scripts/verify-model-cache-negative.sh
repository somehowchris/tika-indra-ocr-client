#!/usr/bin/env bash
# Real release-CLI cache checks; no inference, acquisition or performance claim.
set -euo pipefail

if [[ ${1:-} == --inside ]]; then
    shift
    [[ $(id -u) == 65532 && $(id -g) == 65532 ]]
    printf 'effective_uid=%s effective_gid=%s\n' "$(id -u)" "$(id -g)"
    deadline=$((SECONDS + 30))
    until [[ -f /admission/admitted ]]; do
        ((SECONDS < deadline)) || exit 70
        sleep 0.02
    done
    [[ $(cat /admission/admitted) == admitted ]]
    status=0
    /native-cli "$@" || status=$?
    # Retain original metadata before making only this disposable fixture
    # removable by its host owner. Never change the verified-cache mount.
    if [[ -d /fixture/offline/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 ]]; then
        find /fixture/offline -printf 'cache-entry %y %U %G %m %s %P\n'
        find /fixture/offline -type f -exec sha256sum {} +
        find /fixture/offline -type f -exec chmod 0444 {} +
        find /fixture/offline -mindepth 1 -type d -exec chmod 0777 {} +
    fi
    printf 'native_cli_exit=%s\n' "$status"
    exit "$status"
fi

[[ $# == 6 ]] || {
    echo "Usage: $0 IMMUTABLE_GNU_TOOL_IMAGE STATIC_NATIVE_CLI CLI_SHA256 PRODUCER_SHA NEW_OUTPUT CPU_AFFINITY" >&2
    exit 64
}
image=$1; binary=$(realpath -e -- "$2"); expected=$3; producer=$4
output=$5; affinity=$6
[[ $image =~ ^sha256:[0-9a-f]{64}$ && $expected =~ ^[0-9a-f]{64}$ && $producer =~ ^[0-9a-f]{40}$ && $affinity =~ ^[0-9]+$ ]]
[[ -f $binary && ! -L $binary && -x $binary && ! -e $output ]]
[[ $(sha256sum "$binary" | cut -d' ' -f1) == "$expected" ]]
script=$(realpath -e -- "${BASH_SOURCE[0]}")
root=$(git -C "$(dirname "$script")" rev-parse --show-toplevel)
[[ -z $(git -C "$root" status --porcelain) ]] || { echo 'Controller checkout must be clean.' >&2; exit 1; }
mkdir -m 0700 -- "$output"
output=$(realpath -e -- "$output")
mkdir -m 0755 "$output/control" "$output/cases" "$output/fixtures"
mkdir -m 0777 "$output/fixtures/offline"
mkdir -m 0755 "$output/fixtures/denied"
printf 'permission fixture: unchanged\n' > "$output/fixtures/denied/marker.txt"
chmod 0444 "$output/fixtures/denied/marker.txt"
chmod 0555 "$output/fixtures/denied"
printf '[models]\nrevision = "0000000000000000000000000000000000000000"\n' > "$output/fixtures/unsupported.toml"
chmod 0444 "$output/fixtures/unsupported.toml"
sha256sum "$output/fixtures/denied/marker.txt" > "$output/control/denied-before.sha256"
stat -c '%u %g %a %s %n' "$output/fixtures/denied" "$output/fixtures/denied/marker.txt" > "$output/control/denied-before.stat"
printf '%s\n' "$expected" > "$output/control/native-cli.sha256"
printf '%s\n' "$producer" > "$output/control/native-producer.txt"
git -C "$root" rev-parse HEAD > "$output/control/controller-source.txt"
sha256sum "$script" > "$output/control/controller.sha256"
readelf --program-headers "$binary" > "$output/control/native.program-headers.txt"
readelf --dynamic "$binary" > "$output/control/native.dynamic.txt"
! grep -Eq '(^|[[:space:]])INTERP([[:space:]]|$)' "$output/control/native.program-headers.txt"
! grep -Eq '\(NEEDED\)' "$output/control/native.dynamic.txt"
# This is a scoped unchanged cache/command closure, not whole-application parity.
closure=(backend/models.lock.json runtime/Cargo.toml runtime/Cargo.lock runtime/crates/indra-models runtime/crates/indra-server/src/main.rs runtime/crates/indra-server/src/configuration.rs runtime/crates/indra-server/src/error.rs runtime/crates/indra-server/src/commands.rs)
git -C "$root" diff --exit-code "$producer" HEAD -- "${closure[@]}" > "$output/control/cache-command-source.diff"
git -C "$root" ls-tree -r HEAD -- "${closure[@]}" > "$output/control/cache-command-git-blobs.txt"
bounded() { timeout --signal=TERM --kill-after=3s 30s "$@"; }
docker_cmd() { bounded docker "$@"; }
docker_cmd image inspect "$image" > "$output/control/tool-image.json"
jq -e --arg image "$image" 'length==1 and .[0].Id==$image' "$output/control/tool-image.json" >/dev/null
owner=$(printf '%s\n%s\n%s\n' "$output" "$(cat "$output/control/controller-source.txt")" "$$" | sha256sum | cut -d' ' -f1)
active=; attach_pid=
cleanup() {
    local status=$? cleanup_status=0
    trap - EXIT
    if [[ -n $active ]]; then
        docker_cmd inspect "$active" > "$output/control/cleanup-container.json" || cleanup_status=1
        if ((cleanup_status==0)) && jq -e --arg id "$active" --arg image "$image" --arg owner "$owner" 'length==1 and .[0].Id==$id and .[0].Image==$image and .[0].Config.Labels["io.indra.cache-negative.owner"]==$owner' "$output/control/cleanup-container.json" >/dev/null; then
            docker_cmd rm --force "$active" > "$output/control/cleanup-removed.txt" || cleanup_status=1
        else cleanup_status=1; fi
    fi
    if [[ -n $attach_pid ]]; then wait "$attach_pid" || true; fi
    docker_cmd ps --all --no-trunc --filter "label=io.indra.cache-negative.owner=$owner" --format '{{.ID}}' > "$output/control/cleanup-remaining.txt" || cleanup_status=1
    [[ ! -s $output/control/cleanup-remaining.txt ]] || cleanup_status=1
    printf '%s\n' "$cleanup_status" > "$output/control/cleanup-status.txt"
    ((cleanup_status==0)) || status=1
    printf '%s\n' "$status" > "$output/controller-exit.txt"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT
run_case() {
    local name=$1 directory host_pid relative_group group current quota period memory depth=0 code=0
    shift
    directory=$output/cases/$name
    mkdir -m 0755 "$directory" "$directory/admission"
    docker_cmd create --label "io.indra.cache-negative.owner=$owner" --network none --cgroupns private --read-only --no-healthcheck \
        --user 65532:65532 --cpus 1 --cpuset-cpus "$affinity" --memory 512m --memory-swap 512m --pids-limit 64 \
        --ulimit stack=8388608:8388608 --cap-drop ALL --security-opt no-new-privileges \
        --tmpfs /tmp:rw,nosuid,nodev,size=16m,mode=1777 --entrypoint /bin/bash \
        --env HTTP_PROXY= --env HTTPS_PROXY= --env ALL_PROXY= --env NO_PROXY= \
        --env http_proxy= --env https_proxy= --env all_proxy= --env no_proxy= \
        --mount "type=bind,src=$script,dst=/controller,readonly" \
        --mount "type=bind,src=$binary,dst=/native-cli,readonly" \
        --mount "type=bind,src=$output/fixtures,dst=/fixture" \
        --mount "type=bind,src=$directory/admission,dst=/admission,readonly" \
        "$image" /controller --inside "$@" > "$directory/container-id.txt"
    active=$(cat "$directory/container-id.txt")
    [[ $active =~ ^[0-9a-f]{64}$ ]]
    docker_cmd inspect "$active" > "$directory/container-before.json"
    jq -e --arg id "$active" --arg image "$image" --arg owner "$owner" --arg cpu "$affinity" '
      length==1 and (.[0] | .Id==$id and .Image==$image and .Config.User=="65532:65532" and
      .Config.Labels["io.indra.cache-negative.owner"]==$owner and .HostConfig.NetworkMode=="none" and
      .HostConfig.CgroupnsMode=="private" and .HostConfig.Privileged==false and .HostConfig.PidMode=="" and
      .HostConfig.ReadonlyRootfs==true and .HostConfig.NanoCpus==1000000000 and .HostConfig.CpusetCpus==$cpu and
      .HostConfig.Memory==536870912 and .HostConfig.MemorySwap==536870912 and .HostConfig.PidsLimit==64 and
      .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and .HostConfig.SecurityOpt==["no-new-privileges"])' "$directory/container-before.json" >/dev/null
    timeout --signal=TERM --kill-after=3s 50s docker start --attach "$active" > "$directory/stdout.log" 2> "$directory/stderr.log" &
    attach_pid=$!
    local deadline=$((SECONDS+20))
    host_pid=0
    while ((SECONDS<deadline)); do
        docker_cmd inspect "$active" > "$directory/container-running.json"
        host_pid=$(jq -er '.[0].State.Pid' "$directory/container-running.json")
        [[ $host_pid == 0 ]] || break
        sleep 0.02
    done
    [[ $host_pid =~ ^[1-9][0-9]*$ ]]
    jq -e --arg id "$active" --arg image "$image" --arg owner "$owner" --argjson pid "$host_pid" 'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels["io.indra.cache-negative.owner"]==$owner and .State.Running==true and .State.Pid==$pid)' "$directory/container-running.json" >/dev/null
    cp "/proc/$host_pid/cgroup" "$directory/host-process.cgroup"
    cp "/proc/$host_pid/status" "$directory/host-process.status"
    cp "/proc/$host_pid/limits" "$directory/host-process.limits"
    awk '$1=="Uid:" {found++;if($2!=65532||$3!=65532||$4!=65532||$5!=65532)exit 1} END{if(found!=1)exit 1}' "$directory/host-process.status"
    awk '$1=="Gid:" {found++;if($2!=65532||$3!=65532||$4!=65532||$5!=65532)exit 1} END{if(found!=1)exit 1}' "$directory/host-process.status"
    grep -Eq "^Cpus_allowed_list:[[:space:]]+$affinity$" "$directory/host-process.status"
    relative_group=$(awk -F: '$1=="0" {print $3}' "$directory/host-process.cgroup")
    [[ $relative_group == /* && $(basename "$relative_group") == "docker-$active.scope" ]]
    group=$(readlink -e "/sys/fs/cgroup$relative_group")
    for counter in cpu.max cpuset.cpus.effective memory.max memory.swap.max pids.max memory.events; do
        cp "$group/$counter" "$directory/host-$counter"
    done
    read -r quota period < "$directory/host-cpu.max"
    [[ $quota == 100000 && $period == 100000 && $(cat "$directory/host-memory.max") == 536870912 && $(cat "$directory/host-memory.swap.max") == 0 && $(cat "$directory/host-pids.max") == 64 && $(cat "$directory/host-cpuset.cpus.effective") == "$affinity" ]]
    # Read every ancestor against this declared 1 CPU / 512 MiB correctness budget.
    current=$group
    : > "$directory/host-ancestors.txt"
    while :; do
        depth=$((depth+1)); ((depth<=64))
        printf '%s\n' "$current" >> "$directory/host-ancestors.txt"
        if [[ -f $current/cpu.max ]]; then
            read -r quota period < "$current/cpu.max"
            printf 'cpu.max %s %s\n' "$quota" "$period" >> "$directory/host-ancestors.txt"
            [[ $period =~ ^[1-9][0-9]*$ ]]
            [[ $quota == max ]] || { [[ $quota =~ ^[0-9]+$ ]] && ((quota>=period)); }
        fi
        if [[ -f $current/memory.max ]]; then
            read -r memory < "$current/memory.max"
            printf 'memory.max %s\n' "$memory" >> "$directory/host-ancestors.txt"
            [[ $memory == max ]] || { [[ $memory =~ ^[0-9]+$ ]] && ((memory>=536870912)); }
        fi
        [[ $current == /sys/fs/cgroup ]] && break
        current=$(dirname "$current")
        [[ $current == /sys/fs/cgroup || $current == /sys/fs/cgroup/* ]]
    done
    printf 'admitted\n' > "$directory/admission/admitted"
    wait "$attach_pid" || code=$?
    attach_pid=
    printf '%s\n' "$code" > "$directory/attach-exit.txt"
    [[ $code == 0 || $code == 1 || $code == 2 ]]
    docker_cmd inspect "$active" > "$directory/container-final.json"
    jq -e --arg id "$active" --arg image "$image" --arg owner "$owner" --argjson code "$code" 'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels["io.indra.cache-negative.owner"]==$owner and .RestartCount==0 and .State.Running==false and .State.OOMKilled==false and .State.ExitCode==$code)' "$directory/container-final.json" >/dev/null
    docker_cmd rm "$active" > "$directory/removed.txt"
    active=
}
run_case permission-denied --model-dir /fixture/denied models fetch
[[ $(cat "$output/cases/permission-denied/attach-exit.txt") == 1 ]]
grep -Fq 'Permission denied (os error 13)' "$output/cases/permission-denied/stderr.log"
sha256sum --check "$output/control/denied-before.sha256" > "$output/control/denied-after-verification.txt"
stat -c '%u %g %a %s %n' "$output/fixtures/denied" "$output/fixtures/denied/marker.txt" > "$output/control/denied-after.stat"
cmp "$output/control/denied-before.stat" "$output/control/denied-after.stat"
[[ $(find "$output/fixtures/denied" -mindepth 1 | wc -l) == 1 ]]
run_case cold-network-unavailable --model-dir /fixture/offline/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8 --model-connect-timeout-seconds 1 --model-request-timeout-seconds 1 models fetch
[[ $(cat "$output/cases/cold-network-unavailable/attach-exit.txt") == 1 ]]
grep -Fq 'model download request failed' "$output/cases/cold-network-unavailable/stderr.log"
[[ $(find "$output/fixtures/offline" -type f | wc -l) == 1 ]]
[[ -f $output/fixtures/offline/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/.fetch.lock && ! -s $output/fixtures/offline/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/.fetch.lock ]]
run_case unsupported-revision-flag --model-revision 0000000000000000000000000000000000000000 models verify
[[ $(cat "$output/cases/unsupported-revision-flag/attach-exit.txt") == 2 ]]
grep -Fq "unexpected argument '--model-revision'" "$output/cases/unsupported-revision-flag/stderr.log"
run_case unsupported-revision-config --config /fixture/unsupported.toml check-config
[[ $(cat "$output/cases/unsupported-revision-config/attach-exit.txt") == 1 ]]
grep -Fq 'unknown field;' "$output/cases/unsupported-revision-config/stderr.log"
grep -Fq 'revision' "$output/cases/unsupported-revision-config/stderr.log"
! grep -Fq '0000000000000000000000000000000000000000' "$output/cases/unsupported-revision-config/stderr.log"
jq -n --arg producer "$producer" --arg binary "$expected" '{schema_version:1,passed:true,native_producer:$producer,native_binary_sha256:$binary,cases:4,scope:"Real unprivileged cache permission denial, cold offline failure, unsupported revision configuration, with baked immutable content identity; directory names do not select revisions. No acquired foreign-revision snapshot, inference, download or speed claim."}' > "$output/result.json"
