#!/usr/bin/env bash
# Real native-sidecar shutdown and cgroup gate; cluster overhead is not a benchmark.
set -euo pipefail
[[ $# = 2 ]] || { echo 'usage: verify-kubernetes-native.sh ABSOLUTE_HARNESS NEW_OUTPUT' >&2; exit 2; }
[[ $(uname -s) = Linux ]] || { echo 'This gate requires a local Linux Docker daemon and cgroup v2.' >&2; exit 2; }
harness=$(realpath -- "$1")
output=$(realpath -m -- "$2")
[[ -x "$harness" && ! -e "$output" ]] || exit 2
mkdir -p -- "$(dirname "$output")"
mkdir -- "$output"
mkdir -- "$output/tools" "$output/archives"
name="indralifecycle${GITHUB_RUN_ID:-local}${GITHUB_RUN_ATTEMPT:-0}$$${RANDOM}"
name="${name,,}"
[[ "$name" =~ ^[a-z0-9]{1,63}$ ]] || exit 2
export KUBECONFIG="$output/kubeconfig"
context="kind-$name"
node="$name-control-plane"
printf '%s\n' "$name" > "$output/cluster-name.txt"
active=0
completed=0
forward_pid=
log_pid=
watch_pid=
stream_output=$output
kubectl="$output/tools/kubectl"
kind="$output/tools/kind"
declare -A image_ids container_ids
# shellcheck source=lib/kind-cgroup.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/lib/kind-cgroup.sh"

bounded() {
    local limit=$1
    shift
    timeout --signal=TERM --kill-after=5s "$limit" "$@"
}

stop_stream() {
    local variable=$1 pid status running
    pid=${!variable}
    if [[ -n "$pid" ]]; then
        running=$(jobs -pr)
        if [[ $'\n'"$running"$'\n' == *$'\n'"$pid"$'\n'* ]]; then
            # Each PID is a GNU timeout monitor; TERM reaches its command group,
            # and --kill-after bounds an unresponsive streaming command.
            kill -TERM "$pid" 2>/dev/null || true
        fi
        if wait "$pid"; then status=0; else status=$?; fi
        printf '%s\n' "$status" > "$stream_output/$variable-exit-status.txt"
        printf -v "$variable" '%s' ''
    fi
}

cleanup() {
    local status=$?
    trap - EXIT
    printf '%s\n' "$status" > "$output/primary-exit-status.txt"
    stop_stream forward_pid
    stop_stream log_pid
    stop_stream watch_pid
    if (( active )); then
        bounded 30s "$kubectl" --context "$context" --request-timeout=20s get pods -A -o json > "$output/final-pods.json" 2>&1 || true
        bounded 30s "$kubectl" --context "$context" --request-timeout=20s get events -A -o json > "$output/final-events.json" 2>&1 || true
        bounded 600s "$kind" delete cluster --name "$name" > "$output/cleanup.log" 2>&1 || status=1
    fi
    # Archives are reproducible image intermediates; all gate output is retained.
    rm -f -- "$output/archives/native.tar" "$output/archives/tika.tar" || status=1
    printf '%s\n' "$status" > "$output/exit-status.txt"
    if (( status == 0 && completed )); then
        printf '%s\n' 'Both real Kubernetes native-sidecar resource/lifecycle gates passed. No performance claim.' > "$output/result.txt"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Only complete raw watch records are parsed; any unfinished suffix remains in
# the raw evidence file. Kubernetes watch JSON is newline framed.
# https://kubernetes.io/docs/reference/using-api/api-concepts/#efficient-detection-of-changes
watch_records() {
    jq -Rsc --arg uid "$pod_uid" '
      split("\n") | .[:-1] | map(fromjson) |
      map(select(.object.metadata.uid == $uid))
    ' "$run/pod-watch.jsonl"
}

terminal_evidence() {
    local service=$1 id=${container_ids[$1]} cri_status=0
    local watch_count=0 cri_available=false
    if bounded 20s docker exec "$node" crictl inspect "$id" > "$run/$service-terminal-runtime.json" 2> "$run/$service-terminal-runtime.stderr"; then
        cri_available=true
        jq -e --arg id "$id" --arg uid "$pod_uid" '
          .status.id == $id and .status.labels["io.kubernetes.pod.uid"] == $uid and
          .status.state == "CONTAINER_EXITED" and .status.exitCode == 0 and
          .status.reason != "OOMKilled"
        ' "$run/$service-terminal-runtime.json" > /dev/null || cri_status=1
    else
        cri_status=$?
        printf '%s\n' "$cri_status" > "$run/$service-terminal-runtime-command-status.txt"
        cri_status=0
    fi
    # A live/nonzero/OOM container contradicts a clean shutdown regardless of
    # what another evidence channel reports. Unavailable CRI state can fall back
    # to the retained exact-UID watch after kubelet's garbage collection.
    if (( cri_status )); then
        echo "Original $service container did not exit cleanly: $id" >&2
        return 1
    fi
    watch_records > "$run/pod-watch-records.json"
    jq --arg service "$service" --arg id "containerd://$id" '
      [ .[].object.status | ((.containerStatuses // []) + (.initContainerStatuses // []))[] |
        select(.name == $service) |
        ((.state.terminated // empty) + {observed_container_id: (.state.terminated.containerID // .containerID)}),
        ((.lastState.terminated // empty) + {observed_container_id: .lastState.terminated.containerID}) |
        select(.observed_container_id == $id) ]
    ' "$run/pod-watch-records.json" > "$run/$service-terminal-watch.json"
    watch_count=$(jq 'length' "$run/$service-terminal-watch.json")
    jq -e 'all(.[]; .exitCode == 0 and (.signal // 0) == 0 and .reason != "OOMKilled")' \
        "$run/$service-terminal-watch.json" > /dev/null || { echo "Pod watch recorded an abnormal $service exit." >&2; return 1; }
    jq -e --arg service "$service" '
      all(.[].object.status | ((.containerStatuses // []) + (.initContainerStatuses // []))[] |
      select(.name == $service); .restartCount == 0)
    ' "$run/pod-watch-records.json" > /dev/null || { echo "Pod watch recorded a $service restart." >&2; return 1; }
    if [[ "$cri_available" = false ]] && (( watch_count == 0 )); then
        echo "No terminal evidence remains for original $service container $id; refusing to infer success from pod deletion." >&2
        return 1
    fi
    jq -n --arg service "$service" --arg id "$id" --arg uid "$pod_uid" \
        --argjson cri_available "$cri_available" --argjson watch_count "$watch_count" \
        '{service:$service,container_id:$id,pod_uid:$uid,cri_available:$cri_available,matching_terminal_watch_records:$watch_count,exit_zero_no_oom_or_signal:true}' \
        > "$run/$service-terminal-verdict.json"
}

curl --fail --location --show-error --retry 3 --max-time 180 \
    https://github.com/kubernetes-sigs/kind/releases/download/v0.33.0/kind-linux-amd64 --output "$kind"
curl --fail --location --show-error --retry 3 --max-time 180 \
    https://dl.k8s.io/release/v1.36.4/bin/linux/amd64/kubectl --output "$kubectl"
printf '%s  %s\n' \
    aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d "$kind" \
    8b8f088da2dab964f853b38464033b1be15ede2839eca751482357c45abdd05a "$kubectl" \
    | sha256sum --check --status
chmod 0555 "$kind" "$kubectl"
export PATH="$output/tools:$PATH"
bounded 30s docker version > "$output/docker-version.txt"
bounded 30s docker info --format '{{.CgroupDriver}} {{.CgroupVersion}}' > "$output/docker-cgroup-driver.txt"
[[ $(cat "$output/docker-cgroup-driver.txt") = "systemd 2" ]] || { echo 'This gate requires Docker systemd cgroup v2.' >&2; exit 1; }
bounded 30s "$kind" version > "$output/kind-version.txt"
bounded 30s "$kubectl" version --client -o json > "$output/kubectl-version.json"
bounded 30s "$kind" get clusters > "$output/preexisting-clusters.txt"
! grep -Fxq -- "$name" "$output/preexisting-clusters.txt" || { echo 'Refusing an existing cluster' >&2; exit 1; }
cat > "$output/kind.json" <<'JSON'
{"kind":"Cluster","apiVersion":"kind.x-k8s.io/v1alpha4","networking":{"apiServerAddress":"127.0.0.1"},"nodes":[{"role":"control-plane"}]}
JSON
active=1
bounded 480s "$kind" create cluster --name "$name" --config "$output/kind.json" --wait 180s \
    --image kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed \
    > "$output/create.log" 2>&1
bounded 30s docker exec "$node" cat /etc/os-release > "$output/node-os-release.txt"
grep -Eq '^ID="?debian"?$' "$output/node-os-release.txt"
bounded 30s docker exec "$node" containerd --version > "$output/containerd-version.txt"
bash scripts/record-cri-version.sh "$output/cri-version" docker exec "$node" crictl
bounded 30s "$kubectl" --context "$context" --request-timeout=20s version -o json > "$output/cluster-version.json"
jq -e '.serverVersion.gitVersion == "v1.36.4"' "$output/cluster-version.json" > /dev/null
for role in native tika; do
    image=indra-native:dev
    if [[ "$role" = tika ]]; then image=tika-indra:dev; fi
    archive="$output/archives/$role.tar"
    proof="$output/$role-identity"
    bounded 950s bash scripts/verify-daemon-image.sh resolve "$image" "$proof"
    image_ids[$role]=$(jq -er '.config_digest' "$proof/result.json")
    # Keep the tag for kind's import, but verify the saved raw descriptor graph
    # against the immutable resolution to reject a tag change during export.
    # --platform would replace an index with its child and lose the root proof.
    (ulimit -f 8388608; bounded 300s docker image save --output "$archive" "$image")
    bounded 180s sha256sum "$archive" > "$output/$role-archive.sha256"
    bounded 950s bash scripts/verify-daemon-image.sh verify "$archive" \
        "$proof/root-inspect.json" "$proof/platform-inspect.json" "$output/$role-import-identity"
    bounded 180s "$kind" load image-archive --name "$name" "$archive" > "$output/$role-import.log" 2>&1
    bounded 30s docker exec "$node" crictl inspecti "$image" > "$output/$role-imported-image.json"
    jq -e --arg identity "${image_ids[$role]}" '.status.id == $identity' "$output/$role-imported-image.json" > /dev/null
done
node_id=$(bounded 30s docker inspect --format '{{.Id}}' "$node")
node_pid=$(bounded 30s docker inspect --format '{{.State.Pid}}' "$node")
[[ "$node_id" =~ ^[0-9a-f]{64}$ && "$node_pid" =~ ^[1-9][0-9]*$ ]] || exit 1
bounded 10s sudo -n cat "/proc/$node_pid/cgroup" > "$output/node-process.cgroup"
node_owner=$(kind_cgroup_owner "$node_id" "$output/node-process.cgroup") || {
    echo "Cannot bind kind host PID $node_pid to its exact Docker scope; see $output/node-process.cgroup" >&2
    exit 1
}
node_process=$(cat "$output/node-process.cgroup")
node_process="/sys/fs/cgroup${node_process#0::}"
node_cgroup="/sys/fs/cgroup$node_owner"
[[ $(realpath -e -- "$node_cgroup") = "$node_cgroup" && $(realpath -e -- "$node_process") = "$node_process" ]] || exit 1
jq -n --arg id "$node_id" --argjson pid "$node_pid" --arg owner "$node_cgroup" --arg process "$node_process" \
    '{container_id:$id,host_pid:$pid,docker_owned_cgroup:$owner,host_pid_cgroup:$process}' > "$output/node-cgroup-binding.json"
# Retain both the outer scope and PID leaf. A child may legitimately be `max`;
# the exact outer Docker scope must enforce each numeric CPU budget below.
for counter in cpu.max cpuset.cpus.effective memory.max memory.events; do
    bounded 10s sudo -n cat "$node_cgroup/$counter" > "$output/node-owner-initial-$counter"
    bounded 10s sudo -n cat "$node_process/$counter" > "$output/node-process-initial-$counter"
done
cp -- "$output/node-owner-initial-cpuset.cpus.effective" "$output/node-initial-affinity.txt"
for cpus in 1 2; do
    run="$output/cpu-$cpus"
    mkdir -- "$run"
    stream_output=$run
    namespace="indra-cpu-$cpus"
    # Select from the real available CPU IDs; restricted hosts need not expose0/1.
    affinity=$(awk -v needed="$cpus" '
      BEGIN { count=0; output="" }
      { n=split($0,parts,","); for(i=1;i<=n && count<needed;i++) {
          split(parts[i],bounds,"-"); first=bounds[1]+0; last=(bounds[2]=="" ? first : bounds[2]+0);
          for(cpu=first;cpu<=last && count<needed;cpu++) { output=output (count ? "," : "") cpu; count++ }
      }}
      END { if(count!=needed) exit 1; print output }
    ' "$output/node-initial-affinity.txt")
    # This outer cap includes control-plane and replacement-pod overhead.
    bounded 30s docker update --cpus "$cpus" --cpuset-cpus "$affinity" "$node" > "$run/node-update.txt"
    bounded 30s docker inspect "$node" > "$run/node-container.json"
    jq -e --arg id "$node_id" --argjson pid "$node_pid" --argjson cpus "$cpus" --arg affinity "$affinity" '
      length == 1 and .[0].Id == $id and .[0].State.Running and .[0].State.Pid == $pid and
      .[0].HostConfig.NanoCpus == ($cpus * 1000000000) and .[0].HostConfig.CpusetCpus == $affinity
    ' "$run/node-container.json" > /dev/null
    bounded 10s sudo -n cat "/proc/$node_pid/cgroup" > "$run/node-process.cgroup"
    cmp -- "$output/node-process.cgroup" "$run/node-process.cgroup"
    for counter in cpu.max cpuset.cpus.effective memory.max memory.events; do
        bounded 10s sudo -n cat "$node_cgroup/$counter" > "$run/node-$counter"
        bounded 10s sudo -n cat "$node_process/$counter" > "$run/node-process-$counter"
    done
    # Preserve the full host ancestor chain, including any inherited limits.
    ancestor=$node_process
    : > "$run/node-cgroup-ancestors.txt"
    for ((depth=0; depth<64; depth++)); do
        printf '%s\n' "$ancestor" >> "$run/node-cgroup-ancestors.txt"
        for counter in cpu.max cpuset.cpus.effective memory.max; do
            if [[ -f "$ancestor/$counter" ]]; then
                printf '%s ' "$counter" >> "$run/node-cgroup-ancestors.txt"
                bounded 10s sudo -n cat "$ancestor/$counter" >> "$run/node-cgroup-ancestors.txt"
            fi
        done
        [[ $ancestor != /sys/fs/cgroup ]] || break
        ancestor=${ancestor%/*}
        [[ $ancestor = /sys/fs/cgroup || $ancestor = /sys/fs/cgroup/* ]] || exit 1
    done
    [[ $ancestor = /sys/fs/cgroup ]] || exit 1
    bounded 10s sudo -n cat "/proc/$node_pid/cgroup" > "$run/node-process-after.cgroup"
    cmp -- "$run/node-process.cgroup" "$run/node-process-after.cgroup"
    kind_cgroup_quota "$run/node-cpu.max" "$cpus" || {
        echo "Docker-owned kind scope $node_cgroup must enforce exactly $cpus CPU(s); see $run/node-cpu.max, node-container.json and node-cgroup-ancestors.txt" >&2
        exit 1
    }
    actual_affinity=$(cat "$run/node-cpuset.cpus.effective")
    cmp -- "$run/node-cpuset.cpus.effective" "$run/node-process-cpuset.cpus.effective"
    awk -v expected="$cpus" '
      { n=split($0,parts,","); count=0;
        for(i=1;i<=n;i++) { split(parts[i],bounds,"-"); count += (bounds[2]=="" ? 1 : bounds[2]-bounds[1]+1) }
        if(count!=expected) exit 1 }
    ' "$run/node-cpuset.cpus.effective"
    [[ "$actual_affinity" = "$affinity" || ( "$cpus" = 2 && "$actual_affinity" = "${affinity/,/-}" ) ]] || exit 1
    bounded 30s "$kubectl" --context "$context" --request-timeout=20s create namespace "$namespace" -o json > "$run/namespace.json"
    bounded 30s "$kubectl" --context "$context" kustomize "deploy/overlays/${cpus}cpu" > "$run/admitted.yaml"
    bounded 60s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=50s apply -f "$run/admitted.yaml" > "$run/apply.txt"
    bounded 390s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=380s rollout status deployment/tika-indra --timeout=360s > "$run/rollout.txt"
    # Preserve the server PodList resourceVersion. kubectl's generic get printer
    # rebuilds a List with empty metadata; the watch needs the server snapshot RV.
    bounded 30s "$kubectl" --context "$context" --request-timeout=20s get --raw \
        "/api/v1/namespaces/$namespace/pods?labelSelector=app%3Dtika-indra" > "$run/pods-before.json"
    jq -e '.items | length == 1' "$run/pods-before.json" > /dev/null
    pod=$(jq -er '.items[0].metadata.name' "$run/pods-before.json")
    pod_uid=$(jq -er '.items[0].metadata.uid' "$run/pods-before.json")
    resource_version=$(jq -er '.metadata.resourceVersion' "$run/pods-before.json")
    [[ "$pod" =~ ^[a-z0-9][a-z0-9.-]{0,252}$ && "$pod_uid" =~ ^[0-9a-f-]{36}$ && "$resource_version" =~ ^[0-9]+$ ]] || exit 1
    jq -e '.items[0] | (.spec | .terminationGracePeriodSeconds == 450 and
      ([.containers[].name] == ["tika"]) and
      ([.initContainers[].name] == ["models-init", "indra"]) and
      any(.initContainers[]; .name == "indra" and .restartPolicy == "Always")) and
      (.status | all((.containerStatuses + .initContainerStatuses)[]; .restartCount == 0))
    ' "$run/pods-before.json" > /dev/null
    for service in models-init tika indra; do
        id=$(jq -er --arg name "$service" '.items[0].status | [.containerStatuses[], .initContainerStatuses[]] | .[] | select(.name == $name) | .containerID' "$run/pods-before.json")
        [[ "$id" == containerd://* ]] || exit 1
        id=${id#containerd://}
        [[ "$id" =~ ^[0-9a-f]{64}$ ]] || exit 1
        container_ids[$service]=$id
        bounded 30s docker exec "$node" crictl inspect "$id" > "$run/$service-runtime.json"
        jq -e --arg id "$id" --arg uid "$pod_uid" '.status.id == $id and .status.labels["io.kubernetes.pod.uid"] == $uid' "$run/$service-runtime.json" > /dev/null
        # The status display imageRef can be another repository association.
        # Resolve the immutable creation config ID and bind it to the raw OCI
        # config proven above, the original container ID and exact pod UID.
        creation_image=$(jq -er '.info.config.image.image' "$run/$service-runtime.json")
        [[ "$creation_image" =~ ^sha256:[0-9a-f]{64}$ ]] || exit 1
        bounded 30s docker exec "$node" crictl inspecti "$creation_image" > "$run/$service-running-image.json"
        role=native; if [[ "$service" = tika ]]; then role=tika; fi
        bounded 30s bash scripts/verify-cri-image.sh "$output/$role-identity" \
            "$run/$service-runtime.json" "$run/$service-running-image.json" \
            "$id" "$pod_uid" "$run/$service-image-binding"
        if [[ "$service" = models-init ]]; then
            jq -e '.status | .state == "CONTAINER_EXITED" and .exitCode == 0 and .reason != "OOMKilled"' "$run/$service-runtime.json" > /dev/null
            continue
        fi
        pid=$(jq -er 'select(.status.state == "CONTAINER_RUNNING") | .info.pid' "$run/$service-runtime.json")
        [[ "$pid" =~ ^[1-9][0-9]*$ ]] || exit 1
        cgroup=$(bounded 20s docker exec "$node" cat "/proc/$pid/cgroup")
        [[ "$cgroup" == 0::*"$id"* && "$cgroup" != *$'\n'* ]] || exit 1
        cgroup="/sys/fs/cgroup${cgroup#0::}"
        for counter in cpu.max cpuset.cpus.effective memory.max memory.swap.max memory.events; do
            bounded 20s docker exec "$node" cat "$cgroup/$counter" > "$run/$service-$counter"
        done
        read -r quota period < "$run/$service-cpu.max"
        numerator=1; memory=1073741824
        if [[ "$service" = indra ]]; then numerator=3; memory=2147483648; fi
        [[ "$quota" =~ ^[0-9]+$ && "$period" =~ ^[1-9][0-9]*$ ]] || exit 1
        (( quota * 4 == period * cpus * numerator ))
        [[ $(cat "$run/$service-memory.max") = "$memory" ]]
        [[ $(cat "$run/$service-memory.swap.max") = 0 ]]
    done
    # Start from the captured resourceVersion, so an event between the snapshot
    # and watch connection is retained. Exact UID/CRI-ID filters reject replacements.
    watch_url="/api/v1/namespaces/$namespace/pods?watch=1&resourceVersion=$resource_version&fieldSelector=metadata.name%3D$pod&timeoutSeconds=1000"
    (ulimit -f 65536; exec timeout --signal=TERM --kill-after=5s 1020s "$kubectl" --context "$context" --request-timeout=1010s get --raw "$watch_url") \
        > "$run/pod-watch.jsonl" 2> "$run/pod-watch.stderr" &
    watch_pid=$!
    timeout --signal=TERM --kill-after=5s 1020s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=1010s logs --follow "$pod" --all-containers=true --prefix=true \
        > "$run/pod.log" 2>&1 &
    # Read indirectly by stop_stream through its variable-name argument.
    # shellcheck disable=SC2034
    log_pid=$!
    timeout --signal=TERM --kill-after=5s 1020s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=1010s port-forward --address 127.0.0.1 "pod/$pod" 19998:9998 \
        > "$run/port-forward.log" 2>&1 &
    forward_pid=$!
    tunnel_due=$((SECONDS + 30))
    while ! grep -Fq 'Forwarding from 127.0.0.1:19998 -> 9998' "$run/port-forward.log"; do
        if (( SECONDS >= tunnel_due )) || ! kill -0 "$forward_pid" 2>/dev/null; then
            echo 'The owned port-forward did not establish its local listener.' >&2
            exit 1
        fi
        sleep 0.1
    done
    bounded 30s bash -c 'until curl --fail --silent --max-time 1 http://127.0.0.1:19998/readyz >/dev/null; do sleep 0.2; done'
    bounded 950s "$harness" --endpoint 127.0.0.1:19998 --output "$run/lifecycle.json" --cpus "$cpus" \
        kubernetes --context "$context" --namespace "$namespace" --pod "$pod" \
        > "$run/harness.stdout.log" 2> "$run/harness.stderr.log"
    # Let the asynchronous watch observe the delete notification before stopping
    # it. A missing terminal event cannot become a pass without original CRI proof.
    watch_due=$((SECONDS + 15))
    while (( SECONDS < watch_due )); do
        if watch_records | jq -e 'any(.[]; .type == "DELETED")' > /dev/null; then break; fi
        if ! kill -0 "$watch_pid" 2>/dev/null; then break; fi
        sleep 0.1
    done
    stop_stream forward_pid
    stop_stream log_pid
    stop_stream watch_pid
    terminal_evidence tika
    terminal_evidence indra
    bounded 10s sudo -n cat "$node_cgroup/memory.events" > "$run/node-memory.events-after"
    before_oom=$(awk '$1=="oom_kill" {print $2}' "$run/node-memory.events")
    after_oom=$(awk '$1=="oom_kill" {print $2}' "$run/node-memory.events-after")
    [[ "$before_oom" =~ ^[0-9]+$ && "$after_oom" =~ ^[0-9]+$ && "$before_oom" = "$after_oom" ]] || { echo 'Node cgroup recorded an OOM kill during the lifecycle gate.' >&2; exit 1; }
    bounded 30s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=20s get events -o json > "$run/events.json"
    bounded 510s "$kubectl" --context "$context" --namespace "$namespace" --request-timeout=500s delete -f "$run/admitted.yaml" --wait=true --timeout=480s > "$run/removal.txt"
    bounded 510s "$kubectl" --context "$context" --request-timeout=500s delete namespace "$namespace" --wait=true --timeout=480s > "$run/namespace-removal.txt"
done
completed=1
