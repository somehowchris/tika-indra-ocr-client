#!/usr/bin/env bash
# Local Docker controller; shared phase/source/numerical/sample admission gates.
set -euo pipefail
export LC_ALL=C
[[ $# = 4 ]] || { echo 'usage: profile-native-docker.sh BUNDLE MODELS NEW_OUTPUT TOOLS_IMAGE' >&2; exit 2; }
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
bundle=$(realpath -e -- "$1")
models=$(realpath -e -- "$2")
output=$(realpath -m -- "$3")
image=$4
[[ -d $bundle && -d $models && ! -e $output && ! -L $output ]]
for path in "$root" "$bundle" "$models" "$output"; do
    [[ $path != *','* && $path != *$'\n'* ]] || { echo 'Unsupported Docker bind path' >&2; exit 2; }
done
mkdir -p -- "$(dirname "$output")"
mkdir -m 0700 -- "$output"
/bin/bash "$root/scripts/profile-native.sh" --verify-bundle "$bundle" "$output"
timeout --signal=TERM --kill-after=5s 30s docker image inspect -- "$image" > "$output/tools-image.json"
image=$(jq -er 'select(length==1)|.[0]|select(.Os=="linux" and .Architecture=="amd64")|.Id|select(test("^sha256:[0-9a-f]{64}$"))' "$output/tools-image.json")
timeout --signal=TERM --kill-after=5s 30s docker info --format '{{json .}}' > "$output/docker-info.json"
jq -e '.CgroupDriver=="systemd" and .CgroupVersion=="2" and .OSType=="linux"' "$output/docker-info.json" > /dev/null
attest_host_ancestors() {
    local current=$1 cpus=$2 destination=$3 quota period extra memory depth=0
    [[ $current = /sys/fs/cgroup/* && -d $current ]] || return 1
    : > "$destination"
    while :; do
        depth=$((depth+1)); ((depth<=64)) || return 1
        printf '%s\n' "$current" >> "$destination"
        if [[ -f $current/cpu.max ]]; then
            read -r quota period extra < "$current/cpu.max"
            printf 'cpu.max %s %s\n' "$quota" "$period" >> "$destination"
            [[ $period =~ ^[1-9][0-9]*$ && -z $extra ]] || return 1
            if [[ $quota != max ]]; then [[ $quota =~ ^[0-9]+$ ]] && ((quota>=cpus*period)) || return 1; fi
        fi
        if [[ -f $current/memory.max ]]; then
            memory=$(cat "$current/memory.max")
            printf 'memory.max %s\n' "$memory" >> "$destination"
            if [[ $memory != max ]]; then [[ $memory =~ ^[0-9]+$ ]] && ((memory>=3221225472)) || return 1; fi
        fi
        [[ $current = /sys/fs/cgroup ]] && break
        current=$(dirname "$current")
    done
}
lscpu --json > "$output/lscpu.json"
uname -a > "$output/kernel.txt"
cat /proc/cpuinfo > "$output/cpuinfo.txt"
for name in perf_event_paranoid perf_event_max_sample_rate perf_event_mlock_kb kptr_restrict; do
    cat "/proc/sys/kernel/$name" > "$output/$name"
done
sha256sum "$root/scripts/profile-native.sh" "$root/scripts/verify-native-profile-samples.sh" \
    "$root/scripts/profile-native-docker.sh" "$root/docker/profile-tools.Dockerfile" > "$output/controllers.sha256"
mapfile -t available < <(/bin/bash "$root/scripts/profile-native.sh" --available-cpus)
[[ ${#available[@]} -ge 2 ]] || { echo 'Two schedulable CPUs required' >&2; exit 1; }
run_token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
active='' active_name=''
owner_label=org.tika-indra.native-profile
cleanup() {
    status=$?
    trap - EXIT
    cleanup_status=0
    if [[ -n $active_name ]]; then
        if timeout --signal=TERM --kill-after=5s 15s docker inspect "$active_name" > "$output/interrupted-container.json" 2> "$output/interrupted-inspect.stderr"; then
            if jq -e --arg owner "$run_token" --arg label "$owner_label" --arg image "$image" --arg id "$active" \
                'length==1 and (.[0]|.Config.Labels[$label]==$owner and .Image==$image and ($id=="" or .Id==$id))' \
                "$output/interrupted-container.json" > /dev/null; then
                id=$(jq -er '.[0].Id' "$output/interrupted-container.json")
                timeout --signal=TERM --kill-after=5s 30s docker stop --timeout 15 "$id" > "$output/interrupted-stop.log" 2>&1 || cleanup_status=1
                timeout --signal=TERM --kill-after=5s 15s docker rm "$id" >> "$output/interrupted-stop.log" 2>&1 || cleanup_status=1
            else
                echo 'Refused cleanup: container ownership differs' > "$output/interrupted-stop.log"
                cleanup_status=1
            fi
        else
            # A failed create can legitimately leave no container. Daemon errors
            # remain failures; they are never reported as successful cleanup.
            timeout --signal=TERM --kill-after=5s 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' \
                > "$output/interrupted-list.txt" 2>&1 || cleanup_status=1
            [[ ! -s $output/interrupted-list.txt ]] || cleanup_status=1
        fi
    fi
    [[ $status != 0 || $cleanup_status = 0 ]] || status=1
    jq -n --argjson exit "$status" --argjson cleanup "$cleanup_status" \
        '{exit:$exit,cleanup_exit:$cleanup,visual_review:"pending"}' > "$output/status.json" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for cpus in 1 2; do
    affinity=${PROFILE_CPUSET_1:-${available[0]}}
    if [[ $cpus = 2 ]]; then affinity=${PROFILE_CPUSET_2:-${available[0]},${available[1]}}; fi
    for phase in gates criterion tiff math; do
        directory="$output/cpu-$cpus/$phase"
        mkdir -p "$directory"
        # The parent is private to the host controller. Only this exact bind
        # destination is writable by the capability-free container root user.
        chmod 0777 "$directory"
        active_name="indra-profile-$run_token-$cpus-$phase"
        timeout --signal=TERM --kill-after=5s 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$directory/preexisting.txt"
        [[ ! -s $directory/preexisting.txt ]]
        capabilities=()
        # Only the two real sampling phases need perf_event_open permission.
        if [[ $phase = tiff || $phase = math ]]; then capabilities=(--cap-add=PERFMON); fi
        command=(docker create --name "$active_name" --label "$owner_label=$run_token" --pull=never
            --user=0:0 --cgroupns=private --network=none --read-only --cpus="$cpus" --cpuset-cpus="$affinity"
            --memory=3g --memory-swap=3g --pids-limit=128 --cap-drop=ALL "${capabilities[@]}"
            --security-opt=no-new-privileges --ulimit stack=8388608:8388608
            --tmpfs '/tmp:rw,nosuid,nodev,size=64m,mode=1777' 
            --mount "type=bind,src=$root/scripts/profile-native.sh,dst=/work/scripts/profile-native.sh,readonly"
            --mount "type=bind,src=$root/scripts/verify-native-profile-samples.sh,dst=/work/scripts/verify-native-profile-samples.sh,readonly"
            --mount "type=bind,src=$root/docker/profile-tools.Dockerfile,dst=/work/docker/profile-tools.Dockerfile,readonly"
            --mount "type=bind,src=$bundle,dst=/bundle,readonly"
            --mount "type=bind,src=$models,dst=/models,readonly"
            --mount "type=bind,src=$directory,dst=/evidence"
            --entrypoint /usr/bin/timeout "$image" --signal=TERM --kill-after=15s 900s
            /bin/bash /work/scripts/profile-native.sh --phase /bundle /models "$cpus" "$affinity" docker "$phase" /evidence)
        printf '%q ' "${command[@]}" > "$directory/container-command.txt"
        printf '\n' >> "$directory/container-command.txt"
        timeout --signal=TERM --kill-after=5s 30s "${command[@]}" > "$directory/created.txt" 2> "$directory/create.stderr"
        active=$(cat "$directory/created.txt")
        [[ $active =~ ^[a-f0-9]{64}$ ]]
        timeout --signal=TERM --kill-after=5s 15s docker inspect "$active" > "$directory/container-before.json"
        jq -e --arg owner "$run_token" --arg label "$owner_label" --arg image "$image" --arg affinity "$affinity" \
            --argjson cpus "$cpus" --arg phase "$phase" --arg bundle "$bundle" --arg models "$models" --arg evidence "$directory" --arg root "$root" '
          length==1 and (.[0]|.Image==$image and .Config.Labels[$label]==$owner and .Config.User=="0:0" and
          .HostConfig.CgroupnsMode=="private" and .HostConfig.NanoCpus==($cpus*1000000000) and
          .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and
          .HostConfig.PidsLimit==128 and .HostConfig.ReadonlyRootfs==true and .HostConfig.Privileged==false and
          .HostConfig.PidMode=="" and .HostConfig.NetworkMode=="none" and .HostConfig.CapDrop==["ALL"] and
          (.HostConfig.CapAdd//[])==(if ($phase=="tiff" or $phase=="math") then ["PERFMON"] else [] end) and
          .HostConfig.SecurityOpt==["no-new-privileges"] and
          (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608)) and
          .Config.Entrypoint==["/usr/bin/timeout"] and
          (.Mounts|length)==6 and
          ([.Mounts[]|select(.Type=="bind")|{Source,Destination,RW}]|sort_by(.Destination))==
            ([{Source:($root+"/scripts/profile-native.sh"),Destination:"/work/scripts/profile-native.sh",RW:false},
              {Source:($root+"/scripts/verify-native-profile-samples.sh"),Destination:"/work/scripts/verify-native-profile-samples.sh",RW:false},
              {Source:($root+"/docker/profile-tools.Dockerfile"),Destination:"/work/docker/profile-tools.Dockerfile",RW:false},
              {Source:$bundle,Destination:"/bundle",RW:false},{Source:$models,Destination:"/models",RW:false},
              {Source:$evidence,Destination:"/evidence",RW:true}]|sort_by(.Destination)))
        ' "$directory/container-before.json" > "$directory/container-admission.txt"
        status=0
        timeout --signal=TERM --kill-after=30s 930s docker start --attach "$active" \
            > "$directory/container.stdout" 2> "$directory/container.stderr" &
        attach_pid=$!
        admission_started=$SECONDS
        host_pid=0
        while ((SECONDS-admission_started<30)); do
            timeout --signal=TERM --kill-after=2s 2s docker inspect "$active" > "$directory/container-running.json"
            host_pid=$(jq -er '.[0].State.Pid' "$directory/container-running.json")
            [[ $host_pid = 0 ]] || break
            sleep 0.05
        done
        [[ $host_pid =~ ^[1-9][0-9]*$ ]]
        cp "/proc/$host_pid/cgroup" "$directory/host-process.cgroup"
        relative=$(awk -F: '$1=="0" {print $3}' "$directory/host-process.cgroup")
        [[ $relative = /* && $(basename "$relative") = "docker-$active.scope" ]]
        host_group=$(readlink -e "/sys/fs/cgroup$relative")
        attest_host_ancestors "$host_group" "$cpus" "$directory/host-ancestors.txt"
        jq -n --arg id "$active" --argjson pid "$host_pid" --arg cgroup "$host_group" \
            '{admitted:true,container_id:$id,host_pid:$pid,cgroup:$cgroup}' > "$directory/host-admission.next.json"
        mv "$directory/host-admission.next.json" "$directory/host-admission.json"
        wait "$attach_pid" || status=$?
        timeout --signal=TERM --kill-after=5s 15s docker inspect "$active" > "$directory/container.json"
        jq -e '.|length==1 and (.[0]|.State.Running==false and .State.OOMKilled==false and .State.ExitCode==0)' \
            "$directory/container.json" > "$directory/container-result.txt" || status=1
        jq -e '.exit==0 and .resource_capture_exit==0' "$directory/phase-status.json" > "$directory/phase-admission.txt" || status=1
        timeout --signal=TERM --kill-after=5s 15s docker rm "$active" > "$directory/removed.txt"
        active='' active_name=''
        [[ $status = 0 ]] || { echo "Local profile failed: $cpus/$phase; retained $directory" >&2; exit "$status"; }
    done
done
jq -n '{completed:true,cpus:[1,2],phases_per_cpu:4,visual_review:"pending"}' > "$output/completion.json"
