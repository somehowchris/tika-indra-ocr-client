#!/usr/bin/env bash
# Sequential, finite synthetic decoder pilot. Start only after other builds and
# measurements stop; package power/temperature and shared cache can confound it.
set -euo pipefail
if (( $# != 4 )); then
  echo "usage: $0 IMMUTABLE_OR_LOCAL_IMAGE MODEL_DIRECTORY NEW_EVIDENCE_DIRECTORY EXPECTED_SOURCE_COMMIT" >&2
  exit 2
fi
image=$1
models=$(realpath "$2")
evidence=$3
expected_source=$4
mkdir "$evidence"
evidence=$(realpath "$evidence")
image_id=$(docker image inspect --format '{{.Id}}' "$image")
source_commit=$(docker image inspect --format '{{index .Config.Labels "org.opencontainers.image.revision"}}' "$image_id")
if [[ "$source_commit" != "$expected_source" ]]; then
  echo "image source differs from the recorded producing commit" >&2
  exit 1
fi
printf '%s\n' "$image_id" > "$evidence/image-id.txt"
printf '%s\n' "$source_commit" > "$evidence/source-commit.txt"
lscpu --json > "$evidence/lscpu.json"
lscpu -e=CPU,CORE,SOCKET,CACHE,ONLINE > "$evidence/topology.txt"
date -u +%FT%TZ > "$evidence/started.txt"
prefix="indra-math-$(date -u +%Y%m%d%H%M%S)-$$"
owned_cid=""
cleanup() {
  if [[ -n "$owned_cid" ]]; then
    docker rm --force "$owned_cid" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

run() {
  local role=$1 executable=$2
  shift 2
  local output="$profile/$role"
  mkdir "$output"
  chmod a+rwx "$output" # Public synthetic evidence; mapped nonroot container UID.
  local reference_mount=()
  if [[ -d "$profile/reference/tensors" ]]; then
    reference_mount=(--mount "type=bind,src=$profile/reference/tensors,dst=/reference,readonly")
  fi
  docker create --name "$prefix-cpu$threads-$role" --cidfile "$output/container.cid" \
    --network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges \
    --cpus="$threads" --cpuset-cpus="$affinity" --memory=3g --memory-swap=3g \
    --pids-limit=128 --ulimit stack=8388608:8388608 \
    --tmpfs /tmp:rw,noexec,nosuid,size=64m \
    --mount "type=bind,src=$models,dst=/models,readonly" \
    --mount "type=bind,src=$output,dst=/evidence" "${reference_mount[@]}" \
    --entrypoint "$executable" "$image_id" "$@" > "$output/create.stdout" 2> "$output/create.stderr"
  owned_cid=$(<"$output/container.cid")
  docker inspect --format '{{json .HostConfig}}' "$owned_cid" > "$output/limits.json"
  local attached=0
  timeout --signal=TERM --kill-after=5s 120s docker start --attach "$owned_cid" \
    > "$output/stdout.jsonl" 2> "$output/stderr.log" || attached=$?
  printf '%s\n' "$attached" > "$output/attach-status.txt"
  docker inspect --format '{{json .State}}' "$owned_cid" > "$output/state.json"
  local state
  state=$(docker inspect --format '{{.State.Running}} {{.State.ExitCode}} {{.State.OOMKilled}}' "$owned_cid")
  if [[ "$attached" != 0 || "$state" != "false 0 false" ]]; then
    echo "failed $role at $threads CPUs; retain $output" >&2
    return 1
  fi
  docker rm "$owned_cid" > "$output/remove.stdout"
  owned_cid=""
}

for threads in 1 2; do
  profile="$evidence/cpu$threads"
  mkdir "$profile"
  affinity=0
  if (( threads == 2 )); then affinity=0,1; fi
  run reference /ocr /math.png --model-dir /models --math --threads "$threads" \
    --cv-threads "$threads" --batch-size 64 --trace-directory /evidence/tensors --output /evidence/ordinary.txt
  for arm in ordinary compact; do
    run "gate-$arm" /math-compaction --model-dir /models --reference /reference \
      --resources /evidence/resources --threads "$threads" --arm "$arm" --gate
  done
  for (( repetition=0; repetition<10; repetition++ )); do
    arms=(ordinary compact)
    if (( repetition % 2 )); then arms=(compact ordinary); fi
    for arm in "${arms[@]}"; do
      run "r$repetition-$arm" /math-compaction --model-dir /models --reference /reference \
        --resources /evidence/resources --threads "$threads" --arm "$arm" --warmup
    done
  done
done
date -u +%FT%TZ > "$evidence/completed.txt"
echo "All gates and 40 timed arms completed; analyze paired repetitions and retain raw resource scopes."
