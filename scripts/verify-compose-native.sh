#!/usr/bin/env bash
# Real deployment lifecycle gate. This is not a throughput benchmark.
set -euo pipefail
[[ $# = 2 ]] || { echo 'usage: verify-compose-native.sh ABSOLUTE_HARNESS NEW_OUTPUT' >&2; exit 2; }
[[ $(uname -s) = Linux ]] || { echo 'This gate requires a local Linux Docker daemon and cgroup v2.' >&2; exit 2; }
harness=$(realpath -- "$1")
output=$(realpath -m -- "$2")
[[ -x "$harness" && ! -e "$output" ]] || exit 2
mkdir -p -- "$(dirname "$output")"
mkdir -- "$output"
project="indralifecycle${GITHUB_RUN_ID:-local}${GITHUB_RUN_ATTEMPT:-0}$$${RANDOM}"
project="${project,,}"
[[ "$project" =~ ^[a-z0-9]{1,63}$ ]] || exit 2
printf '%s\n' "$project" > "$output/project.txt"
active=0
completed=0
compose=(docker compose --project-name "$project" --file compose.yaml)
declare -A expected_images container_ids

bounded() {
    local limit=$1
    shift
    timeout --signal=TERM --kill-after=5s "$limit" "$@"
}

cleanup() {
    local status=$?
    trap - EXIT
    printf '%s\n' "$status" > "$output/primary-exit-status.txt"
    if (( active )); then
        bounded 30s "${compose[@]}" logs --no-color > "$output/final-containers.log" 2>&1 || true
        # The preflight rejects every existing resource with this project label.
        # Only the new project and its own model-cache volume are removed.
        bounded 600s "${compose[@]}" down --remove-orphans --volumes > "$output/cleanup.log" 2>&1 || status=1
    fi
    printf '%s\n' "$status" > "$output/exit-status.txt"
    if (( status == 0 && completed )); then
        printf '%s\n' 'Both real Compose resource/lifecycle gates passed. No performance claim.' > "$output/result.txt"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
bounded 30s docker version > "$output/docker-version.txt"
bounded 30s docker compose version > "$output/compose-version.txt"
for kind in container network volume; do
    case "$kind" in
        container) command=(docker ps --all --quiet);;
        network) command=(docker network ls --quiet);;
        volume) command=(docker volume ls --quiet);;
    esac
    bounded 30s "${command[@]}" --filter "label=com.docker.compose.project=$project" > "$output/preexisting-$kind.txt"
    [[ ! -s "$output/preexisting-$kind.txt" ]] || { echo "Refusing an existing Compose project: $project" >&2; exit 1; }
done

for cpus in 1 2; do
    run="$output/cpu-$cpus"
    mkdir -- "$run"
    compose=(docker compose --project-name "$project" --file compose.yaml)
    args=(compose --project "$project" --file compose.yaml)
    if (( cpus == 2 )); then
        compose+=(--file compose.2cpu.yaml)
        args+=(--override-file compose.2cpu.yaml)
    fi
    bounded 30s "${compose[@]}" config --format json > "$run/compose.json"
    # Declarative checks are followed by immutable identity and Linux cgroup checks.
    jq -e --argjson cpus "$cpus" '
      (.services.tika.cpus + .services.indra.cpus) == $cpus and
      .services.tika.network_mode == "service:indra" and
      .services.tika.depends_on.indra.condition == "service_healthy" and
      (.services | keys | sort) == ["indra", "models-init", "tika"] and
      all(.volumes[]; (.external // false) == false)
    ' "$run/compose.json" > /dev/null
    port=$(jq -er '.services.indra.ports | select(length == 1) | .[0] |
      select(.target == 9998 and .host_ip == "127.0.0.1" and .protocol == "tcp") | .published' "$run/compose.json")
    [[ "$port" =~ ^[1-9][0-9]{0,4}$ ]] && (( port <= 65535 ))
    for service in models-init indra tika; do
        image=$(jq -er --arg service "$service" '.services[$service].image | select(type == "string" and length > 0)' "$run/compose.json")
        bounded 30s docker image inspect --platform linux/amd64 -- "$image" > "$run/$service-image.json"
        identity=$(jq -er 'select(length == 1) | .[0] | select(.Os == "linux" and .Architecture == "amd64") | .Id' "$run/$service-image.json")
        [[ "$identity" =~ ^sha256:[0-9a-f]{64}$ ]] || exit 1
        if (( cpus == 1 )); then expected_images[$service]=$identity; fi
        [[ "$identity" = "${expected_images[$service]}" ]] || { echo 'Rendered image changed between CPU gates.' >&2; exit 1; }
    done
    [[ "${expected_images[models-init]}" = "${expected_images[indra]}" ]] || exit 1
    active=1
    bounded 420s "${compose[@]}" up --detach --no-build --pull never --wait --wait-timeout 360 > "$run/startup.log" 2>&1
    bounded 30s "${compose[@]}" ps --all --format json > "$run/containers.json"
    for service in models-init indra tika; do
        id=$(bounded 30s "${compose[@]}" ps --all --quiet "$service")
        [[ "$id" =~ ^[0-9a-f]{64}$ ]] || exit 1
        container_ids[$service]=$id
        bounded 30s docker inspect "$id" > "$run/$service-before.json"
        jq -e --arg identity "${expected_images[$service]}" --arg project "$project" --arg service "$service" '
          length == 1 and (.[0] | .Image == $identity and .RestartCount == 0 and
          .Config.Labels["com.docker.compose.project"] == $project and
          .Config.Labels["com.docker.compose.service"] == $service)
        ' "$run/$service-before.json" > /dev/null
        if [[ "$service" = models-init ]]; then
            jq -e '.[0].State | .Running == false and .OOMKilled == false and .ExitCode == 0' "$run/$service-before.json" > /dev/null
            continue
        fi
        pid=$(jq -er '.[0].State | select(.Running == true and .OOMKilled == false) | .Pid' "$run/$service-before.json")
        [[ "$pid" =~ ^[1-9][0-9]*$ ]] || exit 1
        cgroup=$(bounded 10s sudo -n cat "/proc/$pid/cgroup")
        [[ "$cgroup" == 0::*"$id"* && "$cgroup" != *$'\n'* ]] || exit 1
        cgroup="/sys/fs/cgroup${cgroup#0::}"
        for counter in cpu.max cpuset.cpus.effective memory.max memory.swap.max memory.events; do
            bounded 10s sudo -n cat "$cgroup/$counter" > "$run/$service-$counter"
        done
        read -r quota period < "$run/$service-cpu.max"
        numerator=1; memory=1073741824
        if [[ "$service" = indra ]]; then numerator=3; memory=2147483648; fi
        [[ "$quota" =~ ^[0-9]+$ && "$period" =~ ^[1-9][0-9]*$ ]] || exit 1
        (( quota * 4 == period * cpus * numerator ))
        [[ $(cat "$run/$service-memory.max") = "$memory" ]]
        [[ $(cat "$run/$service-memory.swap.max") = 0 ]]
    done
    jq -e --arg namespace "container:${container_ids[indra]}" '.[0].HostConfig.NetworkMode == $namespace' "$run/tika-before.json" > /dev/null
    bounded 950s "$harness" --endpoint "127.0.0.1:$port" --output "$run/lifecycle.json" --cpus "$cpus" "${args[@]}" \
        > "$run/harness.stdout.log" 2> "$run/harness.stderr.log"
    bounded 30s "${compose[@]}" logs --no-color > "$run/containers.log" 2>&1
    for service in models-init indra tika; do
        id=${container_ids[$service]}
        bounded 30s docker inspect "$id" > "$run/$service-after.json"
        jq -e --arg identity "${expected_images[$service]}" '
          length == 1 and (.[0] | .Image == $identity and .RestartCount == 0 and
          (.State | .Running == false and .OOMKilled == false and .ExitCode == 0))
        ' "$run/$service-after.json" > /dev/null
    done
    bounded 600s "${compose[@]}" down --remove-orphans --volumes > "$run/removal.log" 2>&1
    active=0
done
completed=1
