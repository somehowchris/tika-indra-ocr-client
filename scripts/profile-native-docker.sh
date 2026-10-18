#!/usr/bin/env bash
# Local Docker controller; shared phase/source/numerical/sample admission gates.
set -euo pipefail
export LC_ALL=C
cell=false
if [[ ${1:-} = --ablation-cell ]]; then
    shift
    [[ $# = 9 ]] || exit 2
    cell=true
    cell_cpu=$5 cell_fixture=$6 cell_cap=$7 cell_spin=$8 cell_lto=$9
    [[ $cell_cpu = 1 || $cell_cpu = 2 ]] || exit 2
    [[ $cell_fixture = tiff || $cell_fixture = math || $cell_fixture = gates ]] || exit 2
    [[ $cell_cap = 1 || $cell_cap = 8 || $cell_cap = 64 ]] || exit 2
    [[ $cell_spin = true || $cell_spin = false ]] || exit 2
    [[ $cell_lto = thin || $cell_lto = fat ]] || exit 2
    set -- "$1" "$2" "$3" "$4"
fi
[[ $# = 4 || ( $# = 5 && $5 = --gates-only ) ]] || { echo 'usage: profile-native-docker.sh BUNDLE MODELS NEW_OUTPUT TOOLS_IMAGE [--gates-only]' >&2; exit 2; }
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
bundle=$(realpath -e -- "$1")
models=$(realpath -e -- "$2")
output=$(realpath -m -- "$3")
image=$4
mode=profile
if [[ $# = 5 ]]; then mode=gates; fi
source_root=$(realpath -e -- "${PROFILE_SOURCE_ROOT:-$root}")
[[ -d $bundle && -d $models && ! -e $output && ! -L $output ]]
for path in "$root" "$source_root" "$bundle" "$models" "$output"; do
    [[ $path != *','* && $path != *$'\n'* ]] || { echo 'Unsupported Docker bind path' >&2; exit 2; }
done
mkdir -p -- "$(dirname "$output")"
mkdir -m 0700 -- "$output"
verify_arguments=(--verify-bundle "$bundle" "$output")
if [[ $cell = true ]]; then verify_arguments+=("$cell_lto"); fi
/bin/bash "$source_root/scripts/profile-native.sh" "${verify_arguments[@]}"
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
sha256sum "$source_root/scripts/profile-native.sh" "$source_root/scripts/verify-native-profile-samples.sh" \
    "$root/scripts/profile-native-docker.sh" "$root/scripts/lib/profile-volume.jq" \
    "$root/scripts/lib/profile-capabilities.jq" "$source_root/docker/profile-tools.Dockerfile" > "$output/controllers.sha256"
mapfile -t available < <(/bin/bash "$source_root/scripts/profile-native.sh" --available-cpus)
[[ ${#available[@]} -ge 2 ]] || { echo 'Two schedulable CPUs required' >&2; exit 1; }
run_token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
active='' active_name=''
volume="indra-profile-bundle-$run_token"
owner_label=org.tika-indra.native-profile
owned_volume() {
    jq -e --arg name "$volume" --arg owner "$run_token" --arg label "$owner_label" \
        -f "$root/scripts/lib/profile-volume.jq" "$1" > /dev/null
}
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
    if timeout --signal=TERM --kill-after=5s 15s docker volume inspect "$volume" > "$output/cleanup-volume.json" 2> "$output/cleanup-volume-inspect.stderr"; then
        if owned_volume "$output/cleanup-volume.json"; then
            timeout --signal=TERM --kill-after=5s 15s docker volume rm "$volume" > "$output/cleanup-volume-rm.txt" 2>&1 || cleanup_status=1
        else
            printf 'Refused volume cleanup: ownership/driver/options differ\n' > "$output/cleanup-volume-rm.txt"
            cleanup_status=1
        fi
    fi
    timeout --signal=TERM --kill-after=5s 15s docker volume ls --filter "name=^$volume$" --format '{{.Name}}' \
        > "$output/cleanup-volumes-remaining.txt" 2> "$output/cleanup-volume-list.stderr" || cleanup_status=1
    [[ ! -s $output/cleanup-volumes-remaining.txt ]] || cleanup_status=1
    [[ $status != 0 || $cleanup_status = 0 ]] || status=1
    jq -n --argjson exit "$status" --argjson cleanup "$cleanup_status" \
        '{exit:$exit,cleanup_exit:$cleanup,visual_review:"pending",bundle_transport:"owned private local volume"}' > "$output/status.json" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# The host bundle remains private and is never mounted into a root container.
# Only its verified bytes/modes pass through stdin into an owned local volume.
stat -c '%a %u %g' "$bundle" > "$output/host-bundle-owner.before"
[[ -z $(find "$bundle" ! -type d ! -type f -print -quit) ]]
(cd "$bundle" && find . -printf '%y\t%m\t%P\n' | sort) > "$output/host-bundle.inventory"
jq -n --arg source "$source_root" --arg controller "$root" --arg volume "$volume" --arg mode "$mode" \
    '{frozen_source_root:$source,controller_root:$controller,volume:$volume,mode:$mode,ownership:"copy normalized to container root0; host permissions unchanged"}' > "$output/bundle-transport.json"
timeout --signal=TERM --kill-after=5s 15s docker volume ls --filter "name=^$volume$" --format '{{.Name}}' > "$output/preexisting-volumes.txt"
[[ ! -s $output/preexisting-volumes.txt ]]
timeout --signal=TERM --kill-after=5s 15s docker volume create --driver local --label "$owner_label=$run_token" "$volume" > "$output/created-volume.txt"
timeout --signal=TERM --kill-after=5s 15s docker volume inspect "$volume" > "$output/volume-before.json"
owned_volume "$output/volume-before.json"
active_name="indra-profile-seed-$run_token"
timeout --signal=TERM --kill-after=5s 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/preexisting-seed.txt"
[[ ! -s $output/preexisting-seed.txt ]]
timeout --signal=TERM --kill-after=5s 30s docker create --interactive --name "$active_name" --label "$owner_label=$run_token" --pull=never \
    --user=0:0 --cgroupns=private --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges \
    --cpus=1 --cpuset-cpus="${available[0]}" --memory=3g --memory-swap=3g --pids-limit=128 \
    --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m \
    --mount "type=volume,src=$volume,dst=/bundle,volume-nocopy" \
    --entrypoint /usr/bin/timeout "$image" --signal=TERM --kill-after=10s 180s \
    /bin/bash -euo pipefail -c 'umask 077; tar -xf - -C /bundle --no-same-owner --same-permissions; cd /bundle; \
      test -z "$(find . ! -type d ! -type f -print -quit)"; \
      sha256sum --check --strict artifacts.sha256 >&2; \
      (cd relink && sha256sum --check --strict SHA256SUMS) >&2; \
      find . -printf "%y\t%m\t%P\n" | LC_ALL=C sort' > "$output/created-seed.txt"
active=$(cat "$output/created-seed.txt")
[[ $active =~ ^[a-f0-9]{64}$ ]]
timeout --signal=TERM --kill-after=5s 15s docker inspect "$active" > "$output/seed-before.json"
jq -e --arg owner "$run_token" --arg label "$owner_label" --arg image "$image" --arg id "$active" --arg volume "$volume" --arg affinity "${available[0]}" \
  'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels[$label]==$owner and .Config.User=="0:0" and
   .HostConfig.ReadonlyRootfs and (.HostConfig.Privileged|not) and .HostConfig.NetworkMode=="none" and
   .HostConfig.CgroupnsMode=="private" and .HostConfig.PidMode=="" and .HostConfig.CpusetCpus==$affinity and .Config.Entrypoint==["/usr/bin/timeout"] and .Config.OpenStdin==true and
   .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and .HostConfig.SecurityOpt==["no-new-privileges"] and
   .HostConfig.NanoCpus==1000000000 and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and .HostConfig.PidsLimit==128 and
   (.Mounts|length)==1 and .Mounts[0].Type=="volume" and .Mounts[0].Name==$volume and .Mounts[0].Destination=="/bundle" and .Mounts[0].RW==true and .Mounts[0].Driver=="local" and
   .HostConfig.Mounts[0].VolumeOptions.NoCopy==true)' "$output/seed-before.json" > "$output/seed-admission.txt"
timeout --signal=TERM --kill-after=5s 120s tar -cf - -C "$bundle" . \
    | timeout --signal=TERM --kill-after=10s 200s docker start --attach --interactive "$active" \
        > "$output/volume-bundle.inventory" 2> "$output/seed.stderr"
timeout --signal=TERM --kill-after=5s 15s docker inspect "$active" > "$output/seed-final.json"
jq -e --arg id "$active" --arg owner "$run_token" --arg label "$owner_label" --arg image "$image" \
    'length==1 and (.[0]|.Id==$id and .Image==$image and .Config.Labels[$label]==$owner and .State.Running==false and .State.OOMKilled==false and .State.ExitCode==0)' "$output/seed-final.json" > "$output/seed-result.txt"
cmp "$output/host-bundle.inventory" "$output/volume-bundle.inventory"
stat -c '%a %u %g' "$bundle" > "$output/host-bundle-owner.after"
cmp "$output/host-bundle-owner.before" "$output/host-bundle-owner.after"
timeout --signal=TERM --kill-after=5s 15s docker rm "$active" > "$output/removed-seed.txt"
active='' active_name=''
timeout --signal=TERM --kill-after=5s 15s docker volume inspect "$volume" > "$output/volume-seeded.json"
owned_volume "$output/volume-seeded.json"
cpu_counts=(1 2); phases=(gates criterion tiff math)
if [[ $mode = gates ]]; then phases=(gates); fi
if [[ $cell = true ]]; then
    cpu_counts=("$cell_cpu"); phases=(ablation)
    if [[ $cell_fixture = gates ]]; then phases=(gates); fi
fi
for cpus in "${cpu_counts[@]}"; do
    affinity=${PROFILE_CPUSET_1:-${available[0]}}
    if [[ $cpus = 2 ]]; then affinity=${PROFILE_CPUSET_2:-${available[0]},${available[1]}}; fi
    for phase in "${phases[@]}"; do
        directory="$output/cpu-$cpus/$phase"
        mkdir -p "$directory"
        # The parent is private to the host controller. Only this exact bind
        # destination is writable by the capability-free container root user.
        chmod 0777 "$directory"
        timeout --signal=TERM --kill-after=5s 15s docker volume inspect "$volume" > "$directory/bundle-volume.json"
        owned_volume "$directory/bundle-volume.json"
        active_name="indra-profile-$run_token-$cpus-$phase"
        timeout --signal=TERM --kill-after=5s 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$directory/preexisting.txt"
        [[ ! -s $directory/preexisting.txt ]]
        capabilities=()
        # Only the two real sampling phases need perf_event_open permission.
        if [[ $phase = tiff || $phase = math ]]; then capabilities=(--cap-add=PERFMON); fi
        phase_arguments=(--phase /bundle /models "$cpus" "$affinity" docker "$phase" /evidence)
        if [[ $cell = true && $cell_fixture != gates ]]; then
            phase_arguments=(--ablation-phase /bundle /models "$cpus" "$affinity" "$cell_fixture" "$cell_cap" "$cell_spin" /evidence)
        fi
        command=(docker create --name "$active_name" --label "$owner_label=$run_token" --pull=never
            --user=0:0 --cgroupns=private --network=none --read-only --cpus="$cpus" --cpuset-cpus="$affinity"
            --memory=3g --memory-swap=3g --pids-limit=128 --cap-drop=ALL "${capabilities[@]}"
            --security-opt=no-new-privileges --ulimit stack=8388608:8388608
            --tmpfs '/tmp:rw,nosuid,nodev,size=64m,mode=1777'
            --mount "type=bind,src=$source_root/scripts/profile-native.sh,dst=/work/scripts/profile-native.sh,readonly"
            --mount "type=bind,src=$source_root/scripts/verify-native-profile-samples.sh,dst=/work/scripts/verify-native-profile-samples.sh,readonly"
            --mount "type=bind,src=$source_root/docker/profile-tools.Dockerfile,dst=/work/docker/profile-tools.Dockerfile,readonly"
            --mount "type=volume,src=$volume,dst=/bundle,readonly,volume-nocopy"
            --mount "type=bind,src=$models,dst=/models,readonly"
            --mount "type=bind,src=$directory,dst=/evidence"
            --entrypoint /usr/bin/timeout "$image" --signal=TERM --kill-after=15s 900s
            /bin/bash /work/scripts/profile-native.sh "${phase_arguments[@]}")
        printf '%q ' "${command[@]}" > "$directory/container-command.txt"
        printf '\n' >> "$directory/container-command.txt"
        timeout --signal=TERM --kill-after=5s 30s "${command[@]}" > "$directory/created.txt" 2> "$directory/create.stderr"
        active=$(cat "$directory/created.txt")
        [[ $active =~ ^[a-f0-9]{64}$ ]]
        timeout --signal=TERM --kill-after=5s 15s docker inspect "$active" > "$directory/container-before.json"
        jq -L "$root/scripts/lib" -e --arg owner "$run_token" --arg label "$owner_label" --arg image "$image" --arg affinity "$affinity" \
            --argjson cpus "$cpus" --arg phase "$phase" --arg volume "$volume" --arg models "$models" --arg evidence "$directory" --arg root "$source_root" '
          include "profile-capabilities";
          length==1 and (.[0]|.Image==$image and .Config.Labels[$label]==$owner and .Config.User=="0:0" and
          .HostConfig.CgroupnsMode=="private" and .HostConfig.NanoCpus==($cpus*1000000000) and
          .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and
          .HostConfig.PidsLimit==128 and .HostConfig.ReadonlyRootfs==true and .HostConfig.Privileged==false and
          .HostConfig.PidMode=="" and .HostConfig.NetworkMode=="none" and .HostConfig.CapDrop==["ALL"] and
          profile_capabilities($phase) and
          .HostConfig.SecurityOpt==["no-new-privileges"] and
          (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608)) and
          .Config.Entrypoint==["/usr/bin/timeout"] and
          (.Mounts|length)==6 and
          ([.Mounts[]|select(.Type=="volume")|{Name,Destination,RW,Driver}])==[{Name:$volume,Destination:"/bundle",RW:false,Driver:"local"}] and
          ([.HostConfig.Mounts[]|select(.Type=="volume")]|length)==1 and
          ([.HostConfig.Mounts[]|select(.Type=="volume")][0].VolumeOptions.NoCopy)==true and
          ([.Mounts[]|select(.Type=="bind")|{Source,Destination,RW}]|sort_by(.Destination))==
            ([{Source:($root+"/scripts/profile-native.sh"),Destination:"/work/scripts/profile-native.sh",RW:false},
              {Source:($root+"/scripts/verify-native-profile-samples.sh"),Destination:"/work/scripts/verify-native-profile-samples.sh",RW:false},
              {Source:($root+"/docker/profile-tools.Dockerfile"),Destination:"/work/docker/profile-tools.Dockerfile",RW:false},
              {Source:$models,Destination:"/models",RW:false},
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
if [[ $cell = true ]]; then
    jq -n '{completed:true,scope:"one native ablation cell; no sampling or Criterion"}' > "$output/completion.json"
else
    jq -n --arg mode "$mode" --argjson phases "${#phases[@]}" '{completed:true,cpus:[1,2],phases_per_cpu:$phases,mode:$mode,visual_review:"pending"}' > "$output/completion.json"
fi
