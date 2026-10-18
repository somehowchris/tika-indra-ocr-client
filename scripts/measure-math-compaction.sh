#!/usr/bin/env bash
# Finite synthetic decoder pilot. Wait for other builds/measurements to stop.
set -euo pipefail
export LC_ALL=C
[[ $# = 5 || ( $# = 6 && $6 = --gates-only ) ]] || { echo "usage: $0 IMAGE MODELS NEW_EVIDENCE LOCKED_RECEIPT_DIRECTORY EXPECTED_RECEIPT_SHA256 [--gates-only]" >&2; exit 2; }
mode=paired_timing
if [[ $# = 6 ]]; then mode=correctness; fi
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
# shellcheck source=lib/cgroup-admission.sh
source "$root/scripts/lib/cgroup-admission.sh"
bounded() { timeout --signal=TERM --kill-after=5s "$@"; }
image=$1
models=$(realpath -e -- "$2")
evidence=$(realpath -m -- "$3")
receipt=$(realpath -e -- "$4")
receipt_sha=$5
[[ $receipt_sha =~ ^[0-9a-f]{64}$ && -d $models && ! -e $evidence && ! -L $evidence ]]
[[ $(sha256sum "$receipt/receipt.json" | awk '{print $1}') = "$receipt_sha" ]]
expected_source=$(jq -er '.source_commit|select(test("^[0-9a-f]{40}$"))' "$receipt/receipt.json")
for path in "$models" "$evidence" "$receipt"; do [[ $path != *','* && $path != *$'\n'* ]]; done
mkdir -p -- "$(dirname "$evidence")"
mkdir -m 0700 -- "$evidence"
mkdir "$evidence/materials"
host_snapshot() {
    local destination=$1
    mkdir "$destination" || return
    date -u +%FT%TZ > "$destination/utc.txt" || return
    cat /proc/loadavg > "$destination/loadavg.txt" || return
    cat /proc/stat > "$destination/proc-stat.txt" || return
    ps -e -o pid,ppid,psr,pcpu,pmem,stat,comm > "$destination/processes.txt" || return
    for pressure in cpu io memory; do cat "/proc/pressure/$pressure" > "$destination/pressure-$pressure.txt" || return; done
}
host_snapshot "$evidence/host-before"
cp -- "$0" "$evidence/measure-math-compaction.sh"
cp "$root/scripts/lib/cgroup-admission.sh" "$evidence/cgroup-admission.sh"
files=(protocol.json Cargo.lock native-build-tools.lock native-static-archives.sha256 glibc-archives.txt native-build-identity.txt cargo-env.sh native-link-inputs.sha256 models.lock.json models.sha256 models-verification.log source.tar build.log)
jq -e --argjson names "$(printf '%s\n' "${files[@]}" | jq -Rsc 'split("\n")[:-1]|sort')" '
 .schema_version==1 and .kind=="caller_locked_local_relink_receipt" and
 (.elf_sha256|test("^[0-9a-f]{64}$")) and (.files|keys|sort)==$names and
 (.files|all(.[];test("^[0-9a-f]{64}$")))' "$receipt/receipt.json" >/dev/null
for file in "${files[@]}"; do
    actual=$(sha256sum "$receipt/$file" | awk '{print $1}')
    [[ $(jq -er --arg name "$file" '.files[$name]' "$receipt/receipt.json") = "$actual" ]]
    cp "$receipt/$file" "$evidence/materials/$file"
done
cp "$receipt/receipt.json" "$evidence/materials/receipt.json"
printf '%s\n' "$receipt_sha" > "$evidence/materials/receipt.sha256"
cmp "$evidence/materials/protocol.json" "$root/runtime/validation/math-compaction-protocol.json"
cmp "$evidence/materials/models.lock.json" "$root/backend/models.lock.json"
jq -er '.files|sort_by(.name)[]|select(.name|test("^[A-Za-z0-9._-]+$"))|"\(.sha256)  \(.name)"' "$evidence/materials/models.lock.json" > "$evidence/expected-models.sha256"
[[ $(wc -l < "$evidence/expected-models.sha256") = 14 ]]
cmp "$evidence/expected-models.sha256" "$evidence/materials/models.sha256"
(cd "$models" && sha256sum --check "$evidence/expected-models.sha256") > "$evidence/models-verification.log"
bounded 30s docker image inspect -- "$image" > "$evidence/image.json"
image=$(jq -er 'select(length==1)|.[0]|select(.Os=="linux" and .Architecture=="amd64")|.Id|select(test("^sha256:[0-9a-f]{64}$"))' "$evidence/image.json")
jq -e --arg commit "$expected_source" '.[0].Config.Labels["org.opencontainers.image.revision"]==$commit' "$evidence/image.json" >/dev/null
bounded 30s docker info --format '{{json .}}' > "$evidence/docker-info.json"
jq -e '.CgroupDriver=="systemd" and .CgroupVersion=="2" and .OSType=="linux"' "$evidence/docker-info.json" >/dev/null
lscpu --json > "$evidence/lscpu.json"
lscpu -e=CPU,CORE,SOCKET,CACHE,ONLINE > "$evidence/topology.txt"
lscpu --json --extended=CPU,CORE,SOCKET,CACHE,ONLINE > "$evidence/topology.json"
jq -e '
 [.cpus[]|select(.cpu==0 or .cpu==1)]|sort_by(.cpu)|length==2 and
 (.[0].cpu==0 and .[1].cpu==1 and all(.[];.online==true) and
 .[0].core!=.[1].core and .[0].socket==.[1].socket and
 (.[0]["l1d:l1i:l2:l3"]|split(":")|last|test("^[0-9]+$")) and
 (.[0]["l1d:l1i:l2:l3"]|split(":")|last)==(.[1]["l1d:l1i:l2:l3"]|split(":")|last))
' "$evidence/topology.json" > "$evidence/topology-admission.txt"
sha256sum "$0" "$root/scripts/lib/cgroup-admission.sh" > "$evidence/controllers.sha256"
date -u +%FT%TZ > "$evidence/started.txt"
run_token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
owner_label=org.tika-indra.math-compaction
active='' active_name=''
owned() {
    jq -e --arg token "$run_token" --arg label "$owner_label" --arg image "$image" --arg id "$active" \
      'length==1 and (.[0]|.Config.Labels[$label]==$token and .Image==$image and ($id=="" or .Id==$id))' "$1" >/dev/null
}
cleanup() {
    local status=$? cleanup_status=0 id
    trap - EXIT
    if [[ -n $active_name ]]; then
        if bounded 15s docker inspect "$active_name" > "$evidence/interrupted.json" 2> "$evidence/interrupted-inspect.stderr"; then
            if owned "$evidence/interrupted.json"; then
                id=$(jq -er '.[0].Id' "$evidence/interrupted.json")
                bounded 20s docker stop --timeout 5 "$id" > "$evidence/interrupted-stop.log" 2>&1 || cleanup_status=1
                if bounded 15s docker inspect "$id" > "$evidence/interrupted-final.json" 2>> "$evidence/interrupted-stop.log"; then
                    owned "$evidence/interrupted-final.json" || cleanup_status=1
                    jq -e '.[0].State.Running==false' "$evidence/interrupted-final.json" >/dev/null || cleanup_status=1
                else cleanup_status=1; fi
                bounded 15s docker rm "$id" >> "$evidence/interrupted-stop.log" 2>&1 || cleanup_status=1
            else
                echo 'Refused cleanup: container ID, owner label or image differs' > "$evidence/interrupted-stop.log"
                cleanup_status=1
            fi
        fi
        bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' \
            > "$evidence/cleanup-remaining.txt" 2> "$evidence/cleanup-list.stderr" || cleanup_status=1
        [[ ! -s $evidence/cleanup-remaining.txt ]] || cleanup_status=1
    fi
    host_snapshot "$evidence/host-after" || cleanup_status=1
    [[ $status != 0 || $cleanup_status = 0 ]] || status=1
    jq -n --argjson exit "$status" --argjson cleanup "$cleanup_status" --arg mode "$mode" '{exit:$exit,cleanup_exit:$cleanup,mode:$mode}' > "$evidence/status.json" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

run() {
    local role=$1
    shift
    local output="$profile/$role" attached=0 host_pid=0 started relative host_group effective
    local reference_mount=()
    mkdir "$output"
    chmod a+rwx "$output" # Public fixture evidence; private 0700 parent.
    if [[ -d $profile/reference/tensors ]]; then
        reference_mount=(--mount "type=bind,src=$profile/reference/tensors,dst=/reference,readonly")
    fi
    active_name="indra-math-$run_token-$threads-$role"
    bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/preexisting.txt"
    [[ ! -s $output/preexisting.txt ]]
    bounded 30s docker create --name "$active_name" --label "$owner_label=$run_token" --pull=never \
        --user=65532:65532 --cgroupns=private --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges \
        --cpus="$threads" --cpuset-cpus="$affinity" --memory=3g --memory-swap=3g --pids-limit=128 \
        --ulimit stack=8388608:8388608 --tmpfs /tmp:rw,noexec,nosuid,size=64m \
        --mount "type=bind,src=$models,dst=/models,readonly" --mount "type=bind,src=$output,dst=/evidence" \
        "${reference_mount[@]}" --entrypoint /math-compaction "$image" \
        --host-admission /evidence/host-admitted.txt --model-dir /models --resources /evidence/resources \
        --threads "$threads" "$@" > "$output/created.txt" 2> "$output/create.stderr"
    active=$(<"$output/created.txt")
    [[ $active =~ ^[0-9a-f]{64}$ ]]
    bounded 15s docker inspect "$active" > "$output/container-before.json"
    owned "$output/container-before.json"
    if [[ ! -f $evidence/actual-math-compaction ]]; then
        bounded 30s docker cp "$active:/math-compaction" "$evidence/actual-math-compaction" > "$output/elf-copy.stdout" 2> "$output/elf-copy.stderr"
        actual=$(sha256sum "$evidence/actual-math-compaction" | awk '{print $1}')
        [[ $actual = $(jq -er '.elf_sha256' "$evidence/materials/receipt.json") ]]
        printf '%s  actual-math-compaction\n' "$actual" > "$evidence/executed-elf.sha256"
        bounded 15s readelf --wide --program-headers "$evidence/actual-math-compaction" > "$evidence/elf-program-headers.txt"
        bounded 15s readelf --wide --dynamic "$evidence/actual-math-compaction" > "$evidence/elf-dynamic.txt"
        ! grep -q 'INTERP' "$evidence/elf-program-headers.txt"
        ! grep -q '(NEEDED)' "$evidence/elf-dynamic.txt"
    fi
    jq -e --arg affinity "$affinity" --argjson threads "$threads" \
      '.[0]|.Config.User=="65532:65532" and .Config.Entrypoint==["/math-compaction"] and
       .HostConfig.CgroupnsMode=="private" and .HostConfig.NanoCpus==($threads*1000000000) and
       .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and
       .HostConfig.PidsLimit==128 and .HostConfig.ReadonlyRootfs and (.HostConfig.Privileged|not) and
       .HostConfig.NetworkMode=="none" and .HostConfig.PidMode=="" and .HostConfig.CapDrop==["ALL"] and
       (.HostConfig.CapAdd//[])==[] and .HostConfig.SecurityOpt==["no-new-privileges"] and
       (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608))' "$output/container-before.json" >/dev/null
    bounded 120s docker start --attach "$active" > "$output/stdout.jsonl" 2> "$output/stderr.log" &
    local attach_pid=$!
    started=$SECONDS
    while ((SECONDS-started<20)); do
        bounded 2s docker inspect "$active" > "$output/container-running.json"
        owned "$output/container-running.json"
        host_pid=$(jq -er '.[0].State.Pid' "$output/container-running.json")
        [[ $host_pid = 0 ]] || break
        sleep 0.05
    done
    [[ $host_pid =~ ^[1-9][0-9]*$ ]]
    cp "/proc/$host_pid/cgroup" "$output/host-process.cgroup"
    cp "/proc/$host_pid/status" "$output/host-process.status"
    cp "/proc/$host_pid/limits" "$output/host-process.limits"
    relative=$(awk -F: '$1=="0" {print $3}' "$output/host-process.cgroup")
    [[ $relative = /* && $(basename "$relative") = "docker-$active.scope" ]]
    host_group=$(readlink -e "/sys/fs/cgroup$relative")
    attest_host_ancestors "$host_group" "$threads" "$output/host-ancestors.txt"
    effective=0
    if ((threads==2)); then effective=0-1; fi
    [[ $(awk '/^Cpus_allowed_list:/ {print $2}' "$output/host-process.status") = "$effective" ]]
    [[ $(cat "$host_group/cpuset.cpus.effective") = "$effective" && $(cat "$host_group/pids.max") = 128 ]]
    jq -n --arg id "$active" --argjson pid "$host_pid" --arg cgroup "$host_group" \
        '{admitted:true,container_id:$id,host_pid:$pid,cgroup:$cgroup}' > "$output/host-admission.json"
    printf 'admitted\n' > "$output/host-admitted.next"
    mv "$output/host-admitted.next" "$output/host-admitted.txt"
    wait "$attach_pid" || attached=$?
    printf '%s\n' "$attached" > "$output/attach-status.txt"
    bounded 15s docker inspect "$active" > "$output/container-final.json"
    owned "$output/container-final.json"
    [[ $attached = 0 ]]
    jq -e '.[0].State|.Running==false and .OOMKilled==false and .ExitCode==0' "$output/container-final.json" >/dev/null
    [[ $(awk '$1=="oom_kill" {print $2}' "$output/resources/memory.events.after") = 0 ]]
    bounded 15s docker rm "$active" > "$output/removed.txt"
    bounded 15s docker container ls --all --filter "name=^/$active_name$" --format '{{.ID}}' > "$output/remaining.txt"
    [[ ! -s $output/remaining.txt ]]
    active='' active_name=''
}
for threads in 1 2; do
    profile="$evidence/cpu$threads"
    mkdir "$profile"
    affinity=0
    if ((threads==2)); then affinity=0,1; fi
    run reference --record-reference --reference /evidence/tensors --output /evidence/ordinary.txt
    jq -e '.reference_created==true and .text_bytes==293' "$profile/reference/stdout.jsonl" >/dev/null
    find "$profile/reference/tensors" -type f -print0 | sort -z | xargs -0 sha256sum > "$profile/reference-tensors.sha256"
    for arm in ordinary compact; do
        run "gate-$arm" --reference /reference --arm "$arm" --gate
        jq -e '.gate==true and .timing_valid==false' "$profile/gate-$arm/stdout.jsonl" >/dev/null
    done
    if [[ $mode = correctness ]]; then continue; fi
    for ((repetition=0; repetition<10; repetition++)); do
        arms=(ordinary compact)
        if ((repetition%2)); then arms=(compact ordinary); fi
        for arm in "${arms[@]}"; do
            run "r$repetition-$arm" --reference /reference --arm "$arm" --warmup
            jq -e '.gate==false and .timing_valid==true and .cohort_ns>=.decoder_ns and .decoder_ns>0 and .verified_gather_elements==0' \
                "$profile/r$repetition-$arm/stdout.jsonl" >/dev/null
        done
    done
done
date -u +%FT%TZ > "$evidence/completed.txt"
jq -n --arg mode "$mode" --arg source "$expected_source" '{completed:true,mode:$mode,source:$source,timed_arms:(if $mode=="paired_timing" then 40 else 0 end)}' > "$evidence/completion.json"
echo "Study mode $mode completed; retain every raw attempt and its resource scope."
