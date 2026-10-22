#!/usr/bin/env bash
# Hosted 24-cell OCR matrix controller: verifies the signed candidates, acquires the frozen inputs and runs each arm in an owned cgroup slice.
# jq expressions are passed literally through update_index, then parsed by jq.
# shellcheck disable=SC2016
# Infrastructure only. The locked Rust evaluator owns acquisition, OCR and scoring.
set -Eeuo pipefail
export LC_ALL=C
umask 022

fail() { printf '%s\n' "$*" >&2; return 1; }
sha256_file() { sha256sum -- "$1" | cut -d ' ' -f 1; }
safe_relative() {
  [[ $1 =~ ^[A-Za-z0-9_./-]+$ && $1 != /* && $1 != . && $1 != .. &&
     /$1/ != *'/../'* && /$1/ != *'/./'* && $1 != *'//'* ]]
}
regular_file() { [[ -f $1 && ! -L $1 ]]; }
verify_file() {
  regular_file "$1" && [[ $2 =~ ^[a-f0-9]{64}$ && $3 =~ ^[0-9]+$ ]] &&
    [[ $(stat -c %s -- "$1") == "$3" && $(sha256_file "$1") == "$2" ]]
}
seed_file() {
  local source=$1 root=$2 relative=$3 expected=$4 bytes=$5 parent part
  safe_relative "$relative" || return 1
  verify_file "$source" "$expected" "$bytes" || return 1
  [[ -d $root && ! -L $root ]] || return 1
  parent=$root
  local -a components
  IFS=/ read -r -a components <<< "$relative"
  for part in "${components[@]:0:${#components[@]}-1}"; do
    [[ ! -L $parent/$part ]] || return 1
    mkdir -p -- "$parent/$part" || return 1
    parent=$parent/$part
  done
  if [[ -e $root/$relative || -L $root/$relative ]]; then
    verify_file "$root/$relative" "$expected" "$bytes" || return 1
  else
    # --update=none also protects a destination created after the check above.
    cp --update=none -- "$source" "$root/$relative" || return 1
    verify_file "$root/$relative" "$expected" "$bytes" || return 1
  fi
}

capture() {
  local output=$1 name=$2 seconds=$3 exit_code=0 started ended
  shift 3
  [[ $name =~ ^[A-Za-z0-9_-]+$ && $seconds =~ ^[1-9][0-9]*$ ]] || return 1
  mkdir -p -- "$output/control"
  [[ ! -e $output/control/$name.argv.json ]] || return 1
  jq -n --args '$ARGS.positional' -- "$@" > "$output/control/$name.argv.json"
  started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  timeout --signal=TERM --kill-after=480s "${seconds}s" "$@" \
    > "$output/control/$name.stdout" 2> "$output/control/$name.stderr" || exit_code=$?
  ended=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  jq -n --arg started "$started" --arg ended "$ended" --argjson code "$exit_code" \
    --argjson timeout_seconds "$seconds" \
    '{started_utc:$started,ended_utc:$ended,exit_code:$code,timeout_seconds:$timeout_seconds,timed_out:($code==124 or $code==137)}' \
    > "$output/control/$name.status.json"
  return "$exit_code"
}

# The model cache's fetch lock (.fetch.lock) is a zero-byte file the native
# server leaves in the revision directory, owned by the container user and
# unreadable here; it is not an input and is neither listed nor hashed.
write_evidence_manifest() {
  local root=$1 temporary listing relative valid=true digest
  temporary=$(mktemp "${root%/*}/.evidence-manifest.XXXXXXXX") || return 1
  listing=$(mktemp "${root%/*}/.evidence-list.XXXXXXXX") || { rm -f -- "$temporary"; return 1; }
  if ! (cd "$root" && find . -type f ! -path './SHA256SUMS' ! -path './SHA256SUMS.partial' ! -name .fetch.lock -print | sort) > "$listing"; then
    rm -f -- "$temporary" "$listing"; return 1
  fi
  while IFS= read -r relative; do
    relative=${relative#./}
    safe_relative "$relative" || { valid=false; break; }
    digest=$(sha256_file "$root/$relative") || { valid=false; break; }
    printf '%s  %s\n' "$digest" "$relative" >> "$temporary"
  done < "$listing"
  rm -f -- "$listing"
  [[ $valid == true ]] || { rm -f -- "$temporary"; return 1; }
  mv -- "$temporary" "$root/SHA256SUMS"
}

verify_evidence_manifest() {
  local root=$1 expected actual temporary digest relative previous='' count=0
  regular_file "$root/SHA256SUMS" || return 1
  [[ $(stat -c %s -- "$root/SHA256SUMS") -le 16777216 ]] || return 1
  [[ -z $(find "$root" -type l -print -quit) ]] || return 1
  temporary=$(mktemp "${root%/*}/.manifest-check.XXXXXXXX") || return 1
  while IFS= read -r actual; do
    [[ $actual =~ ^([a-f0-9]{64})\ \ (.+)$ ]] || { rm -f -- "$temporary"; return 1; }
    digest=${BASH_REMATCH[1]}; relative=${BASH_REMATCH[2]}
    safe_relative "$relative" && [[ $relative != SHA256SUMS && $relative != SHA256SUMS.partial && $relative != "$previous" ]] || { rm -f -- "$temporary"; return 1; }
    regular_file "$root/$relative" && [[ $(sha256_file "$root/$relative") == "$digest" ]] || { rm -f -- "$temporary"; return 1; }
    printf '%s\n' "$relative" >> "$temporary"
    previous=$relative; count=$((count + 1))
  done < "$root/SHA256SUMS"
  expected=$(find "$root" -type f ! -path "$root/SHA256SUMS" ! -path "$root/SHA256SUMS.partial" ! -name .fetch.lock -printf '.\n' | wc -l)
  actual=$(sort -u "$temporary" | wc -l)
  rm -f -- "$temporary"
  [[ $count == "$actual" && $count == "$expected" ]]
}

build_schedule() {
  local repository_root=$1 output=$2 laboratory=$3 source_sha=$4 release_id=$5 attempt=$6 budget=$7
  local cohort base manifest protocol seed variant rank index=0
  local -a cohorts=(main)
  [[ $laboratory == true ]] && cohorts+=(laboratory)
  mkdir -p -- "$output/control" "$output/provenance"
  : > "$output/control/studies.jsonl"
  : > "$output/control/arm-order.tsv"
  : > "$output/control/variants.jsonl"
  for cohort in "${cohorts[@]}"; do
    base=evaluation
    [[ $cohort == laboratory ]] && base=evaluation/laboratory-supplement
    manifest=$base/corpus-frozen.json; protocol=$base/protocol.json
    regular_file "$repository_root/$manifest" && regular_file "$repository_root/$protocol" || return 1
    seed=$(jq -er '.selection_seed' "$repository_root/$protocol")
    [[ $(sha256_file "$repository_root/$manifest") == "$(jq -er '.corpus_sha256' "$repository_root/$protocol")" ]] || return 1
    jq -cn --arg cohort "$cohort" --arg manifest "provenance/source/$manifest" --arg protocol "provenance/source/$protocol" \
      --arg manifest_sha256 "$(sha256_file "$repository_root/$manifest")" --arg protocol_sha256 "$(sha256_file "$repository_root/$protocol")" \
      --slurpfile p "$repository_root/$protocol" \
      '{cohort:$cohort,manifest:$manifest,manifest_sha256:$manifest_sha256,protocol:$protocol,protocol_sha256:$protocol_sha256,selection_seed:$p[0].selection_seed,bootstrap_seed:$p[0].metrics.bootstrap_seed,bootstrap_resamples:(if $cohort=="laboratory" then 0 else $p[0].metrics.bootstrap_resamples end)}' \
      >> "$output/control/studies.jsonl"
    jq -er '.primary_variants[].id' "$repository_root/$protocol" > "$output/control/variants-$cohort.txt"
    while IFS= read -r variant; do
      [[ $variant =~ ^[a-z0-9-]+$ ]] || return 1
      rank=$(printf '%s\0%s\0%s\0%s\0' 'indra-evaluation-arm-order-v1' "$seed" "$cohort" "$variant" | sha256sum | cut -d ' ' -f 1)
      printf '%s\t%s\t%s\n' "$cohort" "$rank" "$variant" >> "$output/control/arm-order.tsv"
      jq -cn --arg cohort "$cohort" --arg variant "$variant" --argjson ordinal "$index" \
        '{cohort:$cohort,variant:$variant,ordinal:$ordinal}' >> "$output/control/variants.jsonl"
      index=$((index+1))
    done < "$output/control/variants-$cohort.txt"
  done
  sort -k1,1 -k2,2 "$output/control/arm-order.tsv" > "$output/control/arm-order.sorted.tsv"
  jq -Rn '[inputs|split("\t")|{cohort:.[0],variant:.[2]}]' < "$output/control/arm-order.sorted.tsv" > "$output/control/arm-order.json"
  jq -n --slurpfile arms "$output/control/arm-order.json" --slurpfile studies "$output/control/studies.jsonl" \
    '[1,2][] as $cpus | $studies[] as $study | $arms[0][] | select(.cohort==$study.cohort) | {cohort,variant,cpus:$cpus}' | jq -s 'to_entries|map(.value+{sequence:(.key+1)})' > "$output/provenance/schedule.json"
  jq -n --slurpfile studies "$output/control/studies.jsonl" --slurpfile schedule "$output/provenance/schedule.json" \
    --slurpfile variants "$output/control/variants.jsonl" \
    --arg source_sha "$source_sha" --arg release_run_id "$release_id" --arg release_run_attempt "$attempt" \
    --arg budget "$budget" --arg schedule_sha256 "$(sha256_file "$output/provenance/schedule.json")" \
    '{schema_version:1,status:"partial",source_sha:$source_sha,manifest_sha256:$studies[0].manifest_sha256,protocol_sha256:$studies[0].protocol_sha256,
      assigned_cpu_budget:(if $budget=="all" then null else ($budget|tonumber) end),assigned_budget_complete:false,studies:$studies,
      runs:[$schedule[0][]|.+{status:(if $budget=="all" or .cpus==($budget|tonumber) then "planned" else "not_assigned" end),driver_exit:null,score_exit:null,image_binding_exit:null,reason:null,scores:null,sha256:null,performance:null,performance_sha256:null,attempts:null,attempts_sha256:null,raw_run:null,driver_result:null,driver_result_sha256:null,cgroup:null,parent_before:null,parent_after:null,parent_counters_after:null}],
      comparisons:[$studies[] as $study | [1,2][] as $cpus | $variants[] as $left | $variants[] as $right | select($left.cohort==$study.cohort and $right.cohort==$study.cohort and $left.ordinal<$right.ordinal) | {cohort:$study.cohort,cpus:$cpus,left:$left.variant,right:$right.variant,status:(if $budget=="all" or $cpus==($budget|tonumber) then "planned" else "not_assigned" end),reason:null,exit_code:null,path:null,sha256:null}],
      provenance:{release_run_id:$release_run_id,release_run_attempt:$release_run_attempt,schedule:"provenance/schedule.json",schedule_sha256:$schedule_sha256,hardware_comparison_scope:"Engine comparisons share a host within each CPU budget. Separate budget jobs can use different hardware; do not infer isolated CPU scaling across those hosts."}}' > "$output/study-index.json"
}

update_index() {
  local output=$1 expression=$2
  shift 2
  jq "$@" "$expression" "$output/study-index.json" > "$output/study-index.next.json"
  mv -- "$output/study-index.next.json" "$output/study-index.json"
}

snapshot_inputs() {
  local repository_root=$1 output=$2 source_sha=$3
  mkdir -p -- "$output/provenance/source"
  [[ $(git -C "$repository_root" rev-parse HEAD) == "$source_sha" ]] || return 1
  git -C "$repository_root" diff --quiet HEAD -- evaluation backend/models.lock.json deploy/evaluation scripts .github/workflows/ocr-evaluation.yml || return 1
  git -C "$repository_root" ls-files --error-unmatch scripts/evaluate-ocr-matrix.sh scripts/aggregate-ocr-evaluation.sh .github/workflows/ocr-evaluation.yml >/dev/null || return 1
  git -C "$repository_root" archive --format=tar "$source_sha" \
    evaluation/runner evaluation/acquisition evaluation/audits evaluation/protocol.json \
    evaluation/corpus-frozen.json evaluation/fixtures evaluation/laboratory-supplement \
    backend/models.lock.json deploy/evaluation scripts/evaluate-ocr-matrix.sh \
    scripts/aggregate-ocr-evaluation.sh scripts/test-evaluate-ocr-matrix.sh scripts/docker-evaluation-parent.sh scripts/test-docker-evaluation-parent.sh \
    scripts/verify-daemon-image.sh .github/workflows/ocr-evaluation.yml > "$output/provenance/source.tar"
  tar -xf "$output/provenance/source.tar" -C "$output/provenance/source"
}

cpu_topology_json() {
  # sysfs identity of one logical CPU; absent attributes stay null, malformed ones fail.
  local cpu=$1 root=${MATRIX_SYSFS_CPU_ROOT:-/sys/devices/system/cpu} core='' siblings=''
  [[ $cpu =~ ^[0-9]+$ ]] || return 1
  if regular_file "$root/cpu$cpu/topology/core_id"; then
    read -r core < "$root/cpu$cpu/topology/core_id" || return 1
    [[ $core =~ ^[0-9]+$ ]] || return 1
  fi
  if regular_file "$root/cpu$cpu/topology/thread_siblings_list"; then
    read -r siblings < "$root/cpu$cpu/topology/thread_siblings_list" || return 1
    [[ $siblings =~ ^[0-9]+([,-][0-9]+)*$ ]] || return 1
  fi
  jq -cn --argjson cpu "$cpu" --arg core "$core" --arg siblings "$siblings" \
    '{cpu:$cpu,core_id:(if $core=="" then null else ($core|tonumber) end),thread_siblings_list:(if $siblings=="" then null else $siblings end)}'
}

record_host() {
  local output=$1 allowed first last part cpu entry
  allowed=$(awk '/^Cpus_allowed_list:/ {print $2}' /proc/self/status)
  [[ $allowed =~ ^[0-9,-]+$ ]] || return 1
  local -a available=() ranges
  IFS=, read -r -a ranges <<< "$allowed"
  for part in "${ranges[@]}"; do
    first=${part%-*}; last=${part#*-}
    [[ $first =~ ^[0-9]+$ && $last =~ ^[0-9]+$ && $last -ge $first && $last -le 65535 ]] || return 1
    for ((cpu=first;cpu<=last;cpu++)); do available+=("$cpu"); done
  done
  [[ ${#available[@]} -ge 2 ]] || return 1
  MATRIX_CPU_ONE=${available[0]}
  MATRIX_CPU_TWO=${available[0]},${available[1]}
  # Record what the two selected logical CPUs physically are: on hosted runners
  # they are usually the two hardware threads of one core.
  local -a topology=()
  for cpu in "${available[0]}" "${available[1]}"; do
    entry=$(cpu_topology_json "$cpu") || return 1
    topology+=("$entry")
  done
  jq -n --arg allowed "$allowed" --arg one "$MATRIX_CPU_ONE" --arg two "$MATRIX_CPU_TWO" \
    --argjson topology "$(printf '%s\n' "${topology[@]}" | jq -s '.')" \
    '{process_allowed_cpus:$allowed,selected_cpu_one:$one,selected_cpu_two:$two,selected_cpu_topology:$topology,
      topology_interpretation:"core_id and thread_siblings_list are read from sysfs for each selected logical CPU. When both selected CPUs share one core_id and list each other as siblings, the 2-CPU budget is two hardware threads of one physical core, as on typical 2-vCPU cloud instances; it is not two physical cores.",
      scope:"Actual selected logical CPUs; 1/2-budget jobs may have unlike hosts. Do not interpret cross-host differences as isolated CPU scaling."}' > "$output/provenance/affinity.json"
  capture "$output" host-uname 30 uname -a
  capture "$output" host-lscpu 30 lscpu --json
  capture "$output" host-docker 30 docker info --format '{{json .}}'
  capture "$output" host-systemd 30 systemctl --version
  cp -- /proc/meminfo "$output/provenance/meminfo.txt"
  cp -- /proc/cpuinfo "$output/provenance/cpuinfo.txt"
}

finish_matrix() {
  local original_exit=$1 completed=false
  trap - EXIT TERM INT
  if [[ -n ${MATRIX_OUTPUT:-} && -f $MATRIX_OUTPUT/study-index.json ]]; then
    update_index "$MATRIX_OUTPUT" '.runs |= map(if .status=="running" then .status="interrupted"|.reason="Controller ended before the arm completed; retained evidence is not a repaired attempt." else . end)'
    if [[ -n ${MATRIX_MODEL_CONTAINER:-} ]]; then
      cleanup_model_container "$MATRIX_OUTPUT" || original_exit=1
    fi
    if [[ -n ${MATRIX_ACTIVE_SLICE:-} ]]; then
      release_slice "$MATRIX_OUTPUT" "$MATRIX_ACTIVE_SLICE" "$MATRIX_ACTIVE_UNIT" || original_exit=1
    fi
    completed=$(jq '[.runs[]|select(.status!="not_assigned")|.status=="completed"]|all' "$MATRIX_OUTPUT/study-index.json")
    update_index "$MATRIX_OUTPUT" \
      '.assigned_budget_complete=$complete|.status=(if ([.runs[].status=="completed"]|all) and ([.comparisons[].status=="completed"]|all) then "measured" else "partial" end)|.provenance.controller_exit=$exit|.provenance.ended_utc=$ended' \
      --argjson complete "$completed" --argjson exit "$original_exit" --arg ended "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    write_evidence_manifest "$MATRIX_OUTPUT" || original_exit=1
  fi
  exit "$original_exit"
}

matrix_main() {
  local evaluator='' candidates='' bundle='' repository='' source_sha='' release_id='' attempt='' work='' output='' laboratory=true budget=all resource_backend=systemd helper_image=''
  local script_root name
  script_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
  while (($#)); do
    (($#>=2)) || { fail 'Every matrix option requires a value'; return 2; }
    case $1 in
      --evaluator) evaluator=$2;; --candidates) candidates=$2;; --bundle) bundle=$2;;
      --repository) repository=$2;; --source-sha) source_sha=$2;;
      --release-run-id) release_id=$2;; --release-run-attempt) attempt=$2;;
      --work) work=$2;; --output) output=$2;;
      --include-laboratory) laboratory=$2;; --cpus) budget=$2;;
      --resource-backend) resource_backend=$2;; --helper-image) helper_image=$2;;
      *) fail "Unknown matrix option: $1"; return 2;;
    esac
    shift 2
  done
  [[ $source_sha =~ ^[a-f0-9]{40}$ && $repository =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ &&
     $release_id =~ ^[1-9][0-9]*$ && $attempt =~ ^[1-9][0-9]*$ &&
     $laboratory =~ ^(true|false)$ && $budget =~ ^(1|2|all)$ ]] || { fail 'Invalid matrix identity or budget'; return 2; }
  [[ $resource_backend =~ ^(systemd|docker)$ ]] || return 2
  if [[ $resource_backend == docker ]]; then
    [[ $helper_image =~ ^sha256:[a-f0-9]{64}$ ]] || { fail 'Docker resource setup requires an explicit local GNU helper image ID'; return 2; }
  else
    command -v sudo >/dev/null || return 1
  fi
  MATRIX_RESOURCE_BACKEND=$resource_backend; MATRIX_HELPER_IMAGE=$helper_image
  for name in bash jq sha256sum timeout git tar docker systemctl gh cosign curl readelf; do command -v "$name" >/dev/null || return 1; done
  regular_file "$evaluator" && [[ -x $evaluator ]] && regular_file "$candidates" && regular_file "$bundle" || return 1
  [[ -n $work && -n $output && ! -e $work && ! -e $output && ! -L $work && ! -L $output ]] || return 1
  mkdir -p -- "$work" "$output/provenance" "$output/control"
  work=$(realpath -e -- "$work"); output=$(realpath -e -- "$output")
  MATRIX_OUTPUT=$output
  MATRIX_ACTIVE_SLICE=''; MATRIX_ACTIVE_UNIT=''
  MATRIX_MODEL_CONTAINER=''; MATRIX_MODEL_CLEANUP_ATTEMPT=0
  trap 'finish_matrix $?' EXIT
  trap 'exit 143' TERM
  trap 'exit 130' INT
  cp -- "$evaluator" "$output/provenance/indra-evaluate"
  cp -- "$candidates" "$output/provenance/candidates.json"
  cp -- "$bundle" "$output/provenance/candidates.sigstore.json"
  cp -- "${BASH_SOURCE[0]}" "$output/provenance/evaluate-ocr-matrix.sh"
  evaluator=$output/provenance/indra-evaluate
  candidates=$output/provenance/candidates.json; bundle=$output/provenance/candidates.sigstore.json
  snapshot_inputs "$script_root" "$output" "$source_sha"
  build_schedule "$output/provenance/source" "$output" "$laboratory" "$source_sha" "$release_id" "$attempt" "$budget"
  update_index "$output" \
    '.provenance += {repository:$repository,started_utc:$started,candidates:"provenance/candidates.json",candidates_sha256:$candidates,bundle:"provenance/candidates.sigstore.json",bundle_sha256:$bundle,evaluator:"provenance/indra-evaluate",evaluator_sha256:$evaluator,evaluate_script:"provenance/evaluate-ocr-matrix.sh",evaluate_script_sha256:$script}' \
    --arg repository "$repository" --arg started "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg candidates "$(sha256_file "$candidates")" --arg bundle "$(sha256_file "$bundle")" \
    --arg evaluator "$(sha256_file "$evaluator")" --arg script "$(sha256_file "$output/provenance/evaluate-ocr-matrix.sh")"
  verify_candidates "$output" "$repository" "$source_sha" "$release_id" "$attempt"
  update_index "$output" '.provenance.resource_backend=$backend|.provenance.resource_helper_image=$helper' --arg backend "$resource_backend" --arg helper "$helper_image"
  record_host "$output"
  acquire_inputs "$output" "$work" "$evaluator" "$laboratory"
  execute_schedule "$output" "$work" "$evaluator" "$budget"
  execute_comparisons "$output" "$evaluator" "$budget"
  [[ $(jq '[.runs[]|select(.status!="not_assigned")|.status=="completed"]|all' "$output/study-index.json") == true ]]
}

verify_candidates() {
  local output=$1 repository=$2 source_sha=$3 release_id=$4 attempt=$5 identity source_ref role reference digest name image_id
  local candidates=$output/provenance/candidates.json bundle=$output/provenance/candidates.sigstore.json
  [[ $(stat -c %s -- "$candidates") -le 1048576 && $(stat -c %s -- "$bundle") -le 1048576 ]] || return 1
  jq -e --arg repository "$repository" --arg sha "$source_sha" --arg run "$release_id" --arg attempt "$attempt" '
    .schema_version==1 and .repository==$repository and .source_sha==$sha and
    (.release_run_id|tostring)==$run and (.release_run_attempt|tostring)==$attempt and
    (.source_ref|type=="string" and test("^refs/(heads|tags)/[A-Za-z0-9_./-]+$")) and
    .workflow_ref==($repository+"/.github/workflows/release.yml@"+.source_ref) and
    .workflow_identity==("https://github.com/"+.workflow_ref) and
    .oidc_issuer=="https://token.actions.githubusercontent.com" and
    ([.images[].role]|sort)==["backend","bridge","gateway","native","tesseract","tika"] and
    all(.images[]; (.digest|test("^sha256:[a-f0-9]{64}$")) and .reference==("ghcr.io/"+($repository|split("/")[0]|ascii_downcase)+"/tika-indra-"+.role+"@"+.digest))
  ' "$candidates" >/dev/null || return 1
  capture "$output" release-run-attempt 60 gh api "repos/$repository/actions/runs/$release_id/attempts/$attempt"
  jq -e --arg repository "$repository" --arg sha "$source_sha" --arg run "$release_id" --arg attempt "$attempt" '
    .repository.full_name==$repository and .head_sha==$sha and (.id|tostring)==$run and
    (.run_attempt|tostring)==$attempt and .path==".github/workflows/release.yml" and
    (.event=="push" or .event=="workflow_dispatch")
  ' "$output/control/release-run-attempt.stdout" >/dev/null || return 1
  identity=$(jq -er '.workflow_identity' "$candidates"); source_ref=$(jq -er '.source_ref' "$candidates")
  local -a certificate=(--certificate-identity "$identity" --certificate-oidc-issuer https://token.actions.githubusercontent.com
    --certificate-github-workflow-repository "$repository" --certificate-github-workflow-ref "$source_ref" --certificate-github-workflow-sha "$source_sha")
  capture "$output" candidate-blob-signature 180 cosign verify-blob "${certificate[@]}" --bundle "$bundle" "$candidates"
  jq -r '.images[]|[.role,.reference,.digest]|@tsv' "$candidates" > "$output/control/candidates.tsv"
  : > "$output/control/images.jsonl"
  while IFS=$'\t' read -r role reference digest; do
    name=candidate-$role
    capture "$output" "$name-signature" 180 cosign verify "${certificate[@]}" "$reference"
    capture "$output" "$name-sbom" 180 cosign verify-attestation "${certificate[@]}" --type spdxjson "$reference"
    capture "$output" "$name-pull" 600 docker pull --platform linux/amd64 "$reference"
    capture "$output" "$name-daemon-chain" 600 bash "$output/provenance/source/scripts/verify-daemon-image.sh" resolve "$reference" "$output/provenance/daemon-$role"
    jq -e --arg digest "$digest" '.passed==true and .root_digest==$digest and .image_id==$digest and .os=="linux" and .architecture=="amd64"' \
      "$output/provenance/daemon-$role/identity.json" >/dev/null || return 1
    image_id=$(jq -er '.image_id' "$output/provenance/daemon-$role/identity.json")
    jq -cn --arg role "$role" --arg reference "$reference" --arg image_id "$image_id" \
      --arg resolution "provenance/daemon-$role" --arg resolution_sha256 "$(sha256_file "$output/provenance/daemon-$role/identity.json")" \
      '{role:$role,reference:$reference,image_id:$image_id,resolution:$resolution,resolution_sha256:$resolution_sha256}' >> "$output/control/images.jsonl"
  done < "$output/control/candidates.tsv"
  jq -s '.' "$output/control/images.jsonl" > "$output/provenance/images.json"
  update_index "$output" '.provenance.images="provenance/images.json"|.provenance.images_sha256=$sha' --arg sha "$(sha256_file "$output/provenance/images.json")"
}

seed_legal_inputs() {
  local source=$1 corpus=$2 output=$3 path destination digest bytes
  jq -e '(.files|length)==16 and ([.files[]|select(.corpus_path|endswith(".png"))]|length)==8' "$source/evaluation/fixtures/legal-raster.json" >/dev/null || return 1
  jq -r '.files[]|[.repository_path,.corpus_path,.sha256,.bytes]|@tsv' "$source/evaluation/fixtures/legal-raster.json" > "$output/control/legal-fixtures.tsv"
  while IFS=$'\t' read -r path destination digest bytes; do
    safe_relative "$path" || return 1
    seed_file "$source/$path" "$corpus" "$destination" "$digest" "$bytes" || return 1
  done < "$output/control/legal-fixtures.tsv"
}

cleanup_model_container() {
  local output=$1 name=${MATRIX_MODEL_CONTAINER:-} id running prefix
  [[ -n $name ]] || return 0
  [[ $name =~ ^indra-models-[1-9][0-9]*-(fetch|verify)-[a-f0-9]{32}$ &&
     ${MATRIX_MODEL_TOKEN:-} =~ ^[a-f0-9]{32}$ && ${MATRIX_MODEL_IMAGE:-} =~ ^sha256:[a-f0-9]{64}$ ]] || return 1
  MATRIX_MODEL_CLEANUP_ATTEMPT=$((${MATRIX_MODEL_CLEANUP_ATTEMPT:-0}+1))
  [[ $MATRIX_MODEL_CLEANUP_ATTEMPT -le 2 ]] || return 1
  prefix=models-${MATRIX_MODEL_PHASE}-cleanup-$MATRIX_MODEL_CLEANUP_ATTEMPT
  capture "$output" "$prefix-list" 30 docker container ls --all --no-trunc --filter "name=^/$name$" --format '{{.ID}}' || return 1
  id=$(cat "$output/control/$prefix-list.stdout")
  if [[ -z $id ]]; then MATRIX_MODEL_CONTAINER=''; return 0; fi
  [[ $id =~ ^[a-f0-9]{64}$ ]] || return 1
  capture "$output" "$prefix-inspect" 30 docker container inspect "$id" || return 1
  [[ $(stat -c %s -- "$output/control/$prefix-inspect.stdout") -le 1048576 ]] || return 1
  jq -e --arg name "/$name" --arg id "$id" --arg token "$MATRIX_MODEL_TOKEN" --arg image "$MATRIX_MODEL_IMAGE" \
    'length==1 and .[0].Id==$id and .[0].Name==$name and .[0].Image==$image and
     .[0].Config.Labels["io.indra.evaluation.models"]==$token and (.[0].State.Running|type)=="boolean"' \
    "$output/control/$prefix-inspect.stdout" >/dev/null || return 1
  running=$(jq -er '.[0].State.Running|tostring' "$output/control/$prefix-inspect.stdout")
  if [[ $running == true ]]; then
    capture "$output" "$prefix-stop" 45 docker container stop --time 20 "$id" || return 1
    capture "$output" "$prefix-stopped" 30 docker container inspect "$id" || return 1
    jq -e --arg id "$id" --arg token "$MATRIX_MODEL_TOKEN" \
      'length==1 and .[0].Id==$id and .[0].Config.Labels["io.indra.evaluation.models"]==$token and .[0].State.Running==false' \
      "$output/control/$prefix-stopped.stdout" >/dev/null || return 1
  fi
  # Non-forced removal cannot terminate a process that restarted meanwhile.
  capture "$output" "$prefix-remove" 30 docker container rm "$id" || return 1
  capture "$output" "$prefix-absent" 30 docker container ls --all --no-trunc --filter "name=^/$name$" --format '{{.ID}}' || return 1
  [[ ! -s $output/control/$prefix-absent.stdout ]] || return 1
  MATRIX_MODEL_CONTAINER=''
}

run_model_container() {
  local output=$1 phase=$2 seconds=$3 image_id=$4 code=0 run
  shift 4
  [[ $phase =~ ^(fetch|verify)$ && $image_id =~ ^sha256:[a-f0-9]{64}$ && -z ${MATRIX_MODEL_CONTAINER:-} ]] || return 1
  run=$(jq -er '.provenance.release_run_id' "$output/study-index.json")
  [[ $run =~ ^[1-9][0-9]*$ ]] || return 1
  MATRIX_MODEL_TOKEN=$(tr -d '-' < /proc/sys/kernel/random/uuid)
  [[ $MATRIX_MODEL_TOKEN =~ ^[a-f0-9]{32}$ ]] || return 1
  MATRIX_MODEL_CONTAINER=indra-models-$run-$phase-$MATRIX_MODEL_TOKEN
  MATRIX_MODEL_IMAGE=$image_id; MATRIX_MODEL_PHASE=$phase; MATRIX_MODEL_CLEANUP_ATTEMPT=0
  capture "$output" "models-$phase" "$seconds" docker run --name "$MATRIX_MODEL_CONTAINER" \
    --label "io.indra.evaluation.models=$MATRIX_MODEL_TOKEN" --no-healthcheck "$@" || code=$?
  cleanup_model_container "$output" || return 1
  return "$code"
}

acquire_corpus_inputs() {
  local output=$1 work=$2 evaluator=$3 laboratory=$4 source=$1/provenance/source cohort base id path digest bytes
  mkdir -p -- "$work/main" "$work/laboratory"
  seed_legal_inputs "$source" "$work/main" "$output"
  capture "$output" main-source-fetch 1800 "$evaluator" fetch --lock "$source/evaluation/acquisition/frozen-downloads.json" --root "$work/main"
  capture "$output" main-prepare 900 "$evaluator" prepare --manifest "$source/evaluation/corpus-frozen.json" --root "$work/main" --locks "$source/evaluation/acquisition/locks"
  jq -e '.verified_files==82 and (.failures|length)==0' "$output/control/main-prepare.stdout" >/dev/null || return 1
  local -a cohorts=(main)
  [[ $laboratory == true ]] && cohorts+=(laboratory)
  for cohort in "${cohorts[@]}"; do
    base=$source/evaluation
    [[ $cohort == main ]] || base=$source/evaluation/laboratory-supplement
    if [[ $cohort == laboratory ]]; then
      # Recovered HTML must be exactly the historical bytes. The transparent
      # recovery amendment excludes only two unavailable metadata bodies.
      jq -e --slurpfile original "$base/acquisition/sources.json" --slurpfile audit "$base/audit/metadata-recovery-0002.json" '
        .derived_from.sha256==$audit[0].original_acquisition_lock.sha256 and
        all(.assets[]; . as $asset|any($original[0].assets[]; .==$asset)) and
        ([.assets[]|select(.path|endswith(".pdf"))]==[$original[0].assets[]|select(.path|endswith(".pdf"))])
      ' "$base/acquisition/execution-sources.json" >/dev/null || return 1
      [[ $(sha256_file "$base/acquisition/sources.json") == "$(jq -er '.original_acquisition_lock.sha256' "$base/audit/metadata-recovery-0002.json")" ]] || return 1
      [[ $(sha256_file "$base/acquisition/execution-sources.json") == "$(jq -er '.operational_acquisition_lock.sha256' "$base/audit/metadata-recovery-0002.json")" ]] || return 1
      # The two frozen publisher PDFs are vendored (source-vendoring-0003): the
      # hosted runner received HTTP 403 from the publisher. Their rows come from
      # the operational lock itself, so a vendored byte can never diverge from
      # the locked identity; the evaluator fetch below then only verifies.
      jq -e --slurpfile lock "$base/acquisition/execution-sources.json" '
        .unchanged.original_acquisition_lock.sha256==$lock[0].derived_from.sha256 and .ocr_outputs_used==false and
        ([.vendored_assets[]|{path,sha256,bytes,url}]==[$lock[0].assets[]|select(.path|endswith(".pdf"))|{path,sha256,bytes,url}]) and
        all(.vendored_assets[]; .fixture==("fixtures/"+.path))
      ' "$base/audit/source-vendoring-0003.json" >/dev/null || return 1
      [[ $(sha256_file "$base/acquisition/execution-sources.json") == "$(jq -er '.unchanged.operational_acquisition_lock.sha256' "$base/audit/source-vendoring-0003.json")" ]] || return 1
      {
        jq -r '.assets[]|select(.path|endswith(".pdf"))|["fixtures/"+.path,.path,.sha256,.bytes]|@tsv' "$base/acquisition/execution-sources.json"
        jq -r '.recovered_assets[]|[.fixture,.path,.sha256,.bytes]|@tsv' "$base/audit/metadata-recovery-0002.json"
      } > "$output/control/laboratory-seeded-sources.tsv"
      local archived
      while IFS=$'\t' read -r archived path digest bytes; do
        safe_relative "$archived" && seed_file "$base/$archived" "$work/laboratory" "$path" "$digest" "$bytes" || return 1
      done < "$output/control/laboratory-seeded-sources.tsv"
      capture "$output" laboratory-source-fetch 900 "$evaluator" fetch --lock "$base/acquisition/execution-sources.json" --root "$work/laboratory"
      jq -r '.cases[]|[.id,"input",.input.path,.input.sha256,.input.bytes], [.id,"reference",.reference.path,.reference.sha256,.reference.bytes]|@tsv' "$base/corpus-frozen.json" > "$output/control/laboratory-fixtures.tsv"
      local kind fixture
      while IFS=$'\t' read -r id kind path digest bytes; do
        [[ $id =~ ^[a-z0-9-]+$ ]] || return 1
        fixture=$base/fixtures/$id.png
        [[ $kind == input ]] || fixture=$base/references/$id.txt
        seed_file "$fixture" "$work/laboratory" "$path" "$digest" "$bytes" || return 1
      done < "$output/control/laboratory-fixtures.tsv"
    fi
    [[ $(sha256_file "$base/acquisition/tesseract-models.json") == "$(jq -er '.tesseract_models_lock_sha256' "$base/protocol.json")" ]] || return 1
    capture "$output" "$cohort-tesseract-fetch" 1800 "$evaluator" fetch --lock "$base/acquisition/tesseract-models.json" --root "$work/$cohort" --include-metadata
    capture "$output" "$cohort-corpus-admission" 120 "$evaluator" verify --manifest "$base/corpus-frozen.json" --root "$work/$cohort"
  done
}

acquire_model_inputs() {
  local output=$1 work=$2 source=$1/provenance/source path digest bytes revision native
  mkdir -p -- "$work/models"
  native=$(jq -er '.[]|select(.role=="native")|.image_id' "$output/provenance/images.json")
  revision=$(jq -er '.revision' "$source/backend/models.lock.json")
  [[ $revision =~ ^[a-f0-9]{40}$ ]] || return 1
  if [[ ${MATRIX_RESOURCE_BACKEND:-systemd} == docker ]]; then
    # Only this new empty setup directory is writable during initialization.
    # The parent stays private on the host; serving mounts the verified leaf RO.
    chmod 0700 "$work"
    chmod 0777 "$work/models"
  else
    sudo chown 65532:65532 "$work/models"
  fi
  run_model_container "$output" fetch 900 "$native" --platform linux/amd64 --cpus 2 --cpuset-cpus "$MATRIX_CPU_TWO" \
    --memory 512m --memory-swap 512m --pids-limit 128 --read-only --cap-drop ALL --security-opt no-new-privileges \
    --user 65532:65532 --mount "type=bind,src=$work/models,dst=/models" --tmpfs /tmp:rw,noexec,nosuid,size=32m,uid=65532,gid=65532 \
    --entrypoint /indra-server "$native" --model-dir "/models/$revision" --math true models fetch
  jq -r '.files[]|[.name,.sha256,.size]|@tsv' "$source/backend/models.lock.json" > "$output/control/models.tsv"
  while IFS=$'\t' read -r path digest bytes; do
    safe_relative "$path" && verify_file "$work/models/$revision/$path" "$digest" "$bytes" || return 1
  done < "$output/control/models.tsv"
  run_model_container "$output" verify 180 "$native" --network none --platform linux/amd64 --cpus 2 --cpuset-cpus "$MATRIX_CPU_TWO" \
    --memory 512m --memory-swap 512m --pids-limit 128 --read-only --cap-drop ALL --security-opt no-new-privileges \
    --user 65532:65532 --mount "type=bind,src=$work/models,dst=/models,readonly" \
    --entrypoint /indra-server "$native" --model-dir "/models/$revision" --math true models verify
}

acquire_inputs() {
  local output=$1 work=$2 evaluator=$3 laboratory=$4
  acquire_corpus_inputs "$output" "$work" "$evaluator" "$laboratory"
  acquire_model_inputs "$output" "$work"
  # Exact inventory reads warm the file cache outside the measured parent. The
  # frozen protocol explicitly distinguishes this from storage-cold startup.
  write_evidence_manifest "$work"
  cp -- "$work/SHA256SUMS" "$output/provenance/acquired-inputs.SHA256SUMS"
}

create_slice() {
  local output=$1 sequence=$2 cpus=$3 affinity=$4 nonce run slice unit
  nonce=$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')
  run=$(jq -er '.provenance.release_run_id' "$output/study-index.json")
  [[ $nonce =~ ^[a-f0-9]{16}$ && $run =~ ^[0-9]+$ ]] || return 1
  slice=indraeval${run}n${sequence}${nonce}.slice
  unit=$output/control/$slice
  [[ ! -e /sys/fs/cgroup/$slice && ! -e /run/systemd/system/$slice && ! -L /run/systemd/system/$slice ]] || return 1
  MATRIX_ACTIVE_SLICE=$slice; MATRIX_ACTIVE_UNIT=$unit
  if [[ ${MATRIX_RESOURCE_BACKEND:-systemd} == docker ]]; then
    capture "$output" "slice-$sequence-bootstrap" 180 bash "$output/provenance/source/scripts/docker-evaluation-parent.sh" \
      create "$slice" "$cpus" "$affinity" "$MATRIX_HELPER_IMAGE" "$unit" || return 1
    update_index "$output" '(.runs[]|select(.sequence==$sequence)).cgroup=$cgroup' --argjson sequence "$sequence" --arg cgroup "/sys/fs/cgroup/$slice"
    return 0
  fi
  cat > "$unit" <<EOF
[Unit]
Description=Frozen OCR evaluation $run cell $sequence nonce $nonce
[Slice]
CPUAccounting=yes
MemoryAccounting=yes
CPUQuota=$((cpus*100))%
CPUQuotaPeriodSec=100ms
AllowedCPUs=$affinity
MemoryMax=3221225472
MemorySwapMax=0
TasksMax=384
EOF
  MATRIX_ACTIVE_SLICE=$slice; MATRIX_ACTIVE_UNIT=$unit
  # A flat name without hyphens makes this a direct child of the root slice.
  # Input is an ordinary local file; only the output open needs root privileges.
  # shellcheck disable=SC2024
  sudo bash -c 'set -o noclobber; cat > "$1"' _ "/run/systemd/system/$slice" < "$unit"
  capture "$output" "slice-$sequence-reload" 30 sudo systemctl daemon-reload
  capture "$output" "slice-$sequence-start" 30 sudo systemctl start "$slice"
  capture "$output" "slice-$sequence-identity" 30 sudo systemctl show "$slice" --property=ControlGroup --value
  [[ $(cat "$output/control/slice-$sequence-identity.stdout") == "/$slice" ]] || return 1
  update_index "$output" '(.runs[]|select(.sequence==$sequence)).cgroup=$cgroup' --argjson sequence "$sequence" --arg cgroup "/sys/fs/cgroup/$slice"
}

release_slice() {
  local output=$1 slice=$2 unit=$3 name group=/sys/fs/cgroup/$2
  [[ $slice =~ ^indraeval[0-9]+n[0-9]+[a-f0-9]{16}\.slice$ ]] || return 1
  regular_file "$unit" || return 1
  name=${slice%.slice}
  if [[ ${MATRIX_RESOURCE_BACKEND:-systemd} == docker ]]; then
    jq -e --arg slice "$slice" --arg path "$group" --argjson device "$(stat -c %d "$group")" --argjson inode "$(stat -c %i "$group")" \
      '.slice==$slice and .path==$path and .device==$device and .inode==$inode' "$unit" >/dev/null || return 1
    local sentinel sentinel_id
    sentinel=$(jq -er '.sentinel_name' "$unit"); sentinel_id=$(jq -er '.sentinel_id' "$unit")
    [[ $sentinel == "${slice%.slice}-bootstrap" && $sentinel_id =~ ^[a-f0-9]{64}$ ]] || return 1
    capture "$output" "$name-sentinel-identity" 30 docker container inspect "$sentinel_id" || return 1
    jq -e --arg id "$sentinel_id" --arg slice "$slice" --arg image "$MATRIX_HELPER_IMAGE" \
      'length==1 and .[0].Id==$id and .[0].Image==$image and .[0].HostConfig.CgroupParent==$slice and .[0].Config.Labels["org.tika-indra.evaluation-parent"]==$slice' \
      "$output/control/$name-sentinel-identity.stdout" >/dev/null || return 1
    # Exactly this sentinel may remain after the measured driver finishes.
    grep -Fxq 'nr_descendants 1' "$group/cgroup.stat" || return 1
    capture "$output" "$name-sentinel-remove" 30 docker container rm --force "$sentinel_id" || return 1
    grep -Fxq 'populated 0' "$group/cgroup.events" && grep -Fxq 'nr_descendants 0' "$group/cgroup.stat" || return 1
    cp "$group/cgroup.events" "$output/control/$name.final-cgroup.events"
    # Empty systemd-created slices remain; no host control socket is exposed.
    MATRIX_ACTIVE_SLICE=''; MATRIX_ACTIVE_UNIT=''
    return 0
  fi
  if [[ -d $group ]]; then
    cp -- "$group/cgroup.events" "$output/control/$name.final-cgroup.events"
    if ! awk '$1=="populated" {found=1; if($2!=0) exit 1} END {if(!found) exit 1}' "$group/cgroup.events"; then
      fail "Owned slice remains populated; retained $slice and stopped later arms"
      return 1
    fi
    capture "$output" "$name-stop" 30 sudo systemctl stop "$slice" || return 1
  fi
  if [[ -e /run/systemd/system/$slice || -L /run/systemd/system/$slice ]]; then
    [[ ! -L /run/systemd/system/$slice ]] && sudo cmp --silent -- "$unit" "/run/systemd/system/$slice" || return 1
    sudo rm -- "/run/systemd/system/$slice" || return 1
    capture "$output" "$name-reload" 30 sudo systemctl daemon-reload || return 1
  fi
  MATRIX_ACTIVE_SLICE=''; MATRIX_ACTIVE_UNIT=''
}

bind_executed_images() {
  local output=$1 driver=$2 sequence=$3 engine=$4 role component file
  local -a matches components=(tika)
  [[ $engine == tesseract ]] || components+=(ocr)
  for component in "${components[@]}"; do
    role=tika
    [[ $engine != tesseract ]] || role=tesseract
    if [[ $component == ocr ]]; then
      role=backend
      [[ $engine != rust ]] || role=native
    fi
    shopt -s nullglob
    matches=("$driver"/commands/*-"$component"-created-inspect.stdout)
    shopt -u nullglob
    [[ ${#matches[@]} == 1 ]] || return 1
    file=${matches[0]}
    regular_file "$file" && [[ $(stat -c %s -- "$file") -le 1048576 ]] || return 1
    jq '[.]' "$file" > "$output/control/cell-$sequence-$component-container.json"
    capture "$output" "cell-$sequence-$component-image-binding" 180 bash \
      "$output/provenance/source/scripts/verify-daemon-image.sh" bind "$output/provenance/daemon-$role" \
      "$output/control/cell-$sequence-$component-container.json" "$driver/verified-$component-image" || return 1
  done
}

execute_schedule() {
  local output=$1 work=$2 evaluator=$3 budget=$4 sequence cohort variant cpus base engine flavor psm math threads affinity
  local relative driver scores driver_exit score_exit identity_exit max_seconds count image backend status reason path digest raw_run
  local source=$output/provenance/source
  jq -r '.runs[]|select(.status=="planned")|[.sequence,.cohort,.variant,.cpus]|@tsv' "$output/study-index.json" > "$output/control/assigned-cells.tsv"
  while IFS=$'\t' read -r sequence cohort variant cpus; do
    [[ $budget == all || $budget == "$cpus" ]] || return 1
    base=$source/evaluation
    [[ $cohort == main ]] || base=$source/evaluation/laboratory-supplement
    engine=$(jq -er --arg id "$variant" '.primary_variants[]|select(.id==$id)|.engine' "$base/protocol.json")
    affinity=$MATRIX_CPU_ONE; [[ $cpus == 1 ]] || affinity=$MATRIX_CPU_TWO
    relative=runs/$cohort/cpu$cpus/$variant
    driver=$output/$relative/driver; scores=$output/$relative/scored
    mkdir -p -- "$output/$relative"
    update_index "$output" '(.runs[]|select(.sequence==$sequence)) |= (.status="running"|.reason=null)' --argjson sequence "$sequence"
    create_slice "$output" "$sequence" "$cpus" "$affinity"
    image=$(jq -er --arg role "$(if [[ $engine == tesseract ]]; then printf tesseract; else printf tika; fi)" '.[]|select(.role==$role)|.image_id' "$output/provenance/images.json")
    local -a launcher=(sudo)
    [[ ${MATRIX_RESOURCE_BACKEND:-systemd} != docker ]] || launcher=()
    local -a command=("${launcher[@]}" "$evaluator" docker --evaluator "$evaluator" --tika-image "$image" --corpus "$work/$cohort"
      --manifest "$base/corpus-frozen.json" --protocol "$base/protocol.json" --output "$driver" --label "$variant"
      --cpus "$cpus" --parent-slice "$MATRIX_ACTIVE_SLICE" --parent-cgroup "/sys/fs/cgroup/$MATRIX_ACTIVE_SLICE"
      --affinity "$affinity" --repetitions 3 --batch-size 64)
    [[ ${MATRIX_RESOURCE_BACKEND:-systemd} != docker ]] || command+=(--docker-runner --docker-parent-receipt "$MATRIX_ACTIVE_UNIT")
    if [[ $engine == tesseract ]]; then
      flavor=$(jq -er --arg id "$variant" '.primary_variants[]|select(.id==$id)|.tessdata' "$base/protocol.json")
      psm=$(jq -er --arg id "$variant" '.primary_variants[]|select(.id==$id)|.psm' "$base/protocol.json")
      [[ $flavor =~ ^(fast|best)$ && $psm =~ ^(3|6|11)$ ]] || return 1
      command+=(--arm tesseract --threads 1 --tesseract-psm "$psm" --tika-config "$source/deploy/evaluation/tika-tesseract.xml"
        --models "$work/$cohort/tesseract/$flavor" --tesseract-models-lock "$base/acquisition/tesseract-models.json")
    else
      backend=backend; [[ $engine != rust ]] || backend=native
      backend=$(jq -er --arg role "$backend" '.[]|select(.role==$role)|.image_id' "$output/provenance/images.json")
      math=$(jq -er --arg id "$variant" '.primary_variants[]|select(.id==$id)|.math|tostring' "$base/protocol.json")
      threads=$cpus
      command+=(--arm "$(if [[ $engine == rust ]]; then printf native; else printf python; fi)" --backend-image "$backend"
        --threads "$threads" --tika-config "$source/deploy/evaluation/tika-indra.xml" --models "$work/models/$(jq -er '.revision' "$source/backend/models.lock.json")")
      [[ $math == false ]] || command+=(--math)
    fi
    count=$(jq -er '.cases|length' "$base/corpus-frozen.json")
    max_seconds=$(((count*3+1)*130+1200))
    driver_exit=0; score_exit=null; identity_exit=null
    capture "$output" "cell-$sequence-driver" "$max_seconds" "${command[@]}" || driver_exit=$?
    # The driver retains exact raw Docker inspections. Bind the executed root
    # and selected manifest to the previously verified daemon graph before a
    # cell can become admissible. This does not re-run any OCR request.
    if [[ -f $driver/driver-result.json ]]; then
      identity_exit=0
      bind_executed_images "$output" "$driver" "$sequence" "$engine" || identity_exit=$?
    fi
    if [[ -f $driver/run/requests.jsonl ]]; then
      score_exit=0
      capture "$output" "cell-$sequence-score" 600 "$evaluator" score --manifest "$base/corpus-frozen.json" \
        --root "$work/$cohort" --run "$driver/run" --output "$scores" || score_exit=$?
    fi
    status=failed; reason='Driver, image binding or untimed score admission failed; all first-attempt evidence is retained.'
    if [[ $driver_exit == 0 && $score_exit == 0 && $identity_exit == 0 ]] &&
       jq -e '.complete==true' "$driver/driver-result.json" >/dev/null && jq -e '.journal_complete==true' "$scores/scores.json" >/dev/null; then
      status=completed; reason=''
    fi
    raw_run=''
    [[ ! -d $driver/run ]] || raw_run=$relative/driver/run
    update_index "$output" '(.runs[]|select(.sequence==$sequence)) += {status:$status,reason:(if $reason=="" then null else $reason end),driver_exit:$driver_exit,score_exit:$score_exit,image_binding_exit:$identity_exit,raw_run:(if $raw_run=="" then null else $raw_run end)}' \
      --argjson sequence "$sequence" --arg status "$status" --arg reason "$reason" --argjson driver_exit "$driver_exit" \
      --argjson score_exit "$score_exit" --argjson identity_exit "$identity_exit" --arg raw_run "$raw_run"
    local field hash_field artifact
    for field in scores performance attempts driver_result; do
      case $field in
        scores) artifact=$relative/scored/scores.json; hash_field=sha256;;
        performance) artifact=$relative/scored/performance.json; hash_field=performance_sha256;;
        attempts) artifact=$relative/scored/attempts.csv; hash_field=attempts_sha256;;
        driver_result) artifact=$relative/driver/driver-result.json; hash_field=driver_result_sha256;;
      esac
      if regular_file "$output/$artifact"; then
        digest=$(sha256_file "$output/$artifact")
        update_index "$output" '(.runs[]|select(.sequence==$sequence)) += {($field):$path,($hash_field):$sha}' \
          --argjson sequence "$sequence" --arg field "$field" --arg hash_field "$hash_field" --arg path "$artifact" --arg sha "$digest"
      fi
    done
    for field in parent_before parent_after parent_counters_after; do
      path=$relative/driver/${field//_/-}.json
      if regular_file "$output/$path"; then
        update_index "$output" '(.runs[]|select(.sequence==$sequence))[$field]=$path' --argjson sequence "$sequence" --arg field "$field" --arg path "$path"
      fi
    done
    release_slice "$output" "$MATRIX_ACTIVE_SLICE" "$MATRIX_ACTIVE_UNIT" || return 1
  done < "$output/control/assigned-cells.tsv"
}

execute_comparisons() {
  local output=$1 evaluator=$2 budget=$3 cohort cpus left right seed resamples relative code status reason left_path right_path
  jq -r '.comparisons[]|select(.status=="planned")|[.cohort,.cpus,.left,.right]|@tsv' "$output/study-index.json" > "$output/control/assigned-comparisons.tsv"
  while IFS=$'\t' read -r cohort cpus left right; do
    [[ $budget == all || $budget == "$cpus" ]] || return 1
    relative=comparisons/$cohort/cpu$cpus/$left--$right
    code=null; status=skipped; reason='Only complete first-attempt journals with successful driver and image admission may be paired.'
    left_path=$(jq -r --arg c "$cohort" --arg v "$left" --argjson n "$cpus" '.runs[]|select(.cohort==$c and .variant==$v and .cpus==$n and .status=="completed")|.scores' "$output/study-index.json")
    right_path=$(jq -r --arg c "$cohort" --arg v "$right" --argjson n "$cpus" '.runs[]|select(.cohort==$c and .variant==$v and .cpus==$n and .status=="completed")|.scores' "$output/study-index.json")
    if [[ -n $left_path && -n $right_path ]]; then
      seed=$(jq -er --arg c "$cohort" '.studies[]|select(.cohort==$c)|.bootstrap_seed' "$output/study-index.json")
      resamples=$(jq -er --arg c "$cohort" '.studies[]|select(.cohort==$c)|.bootstrap_resamples' "$output/study-index.json")
      code=0
      capture "$output" "compare-$cohort-$cpus-$left-$right" 180 "$evaluator" compare --left "$output/$left_path" --right "$output/$right_path" \
        --output "$output/$relative" --seed "$seed" --resamples "$resamples" || code=$?
      status=failed; reason='Paired comparison failed; raw scorer diagnostics retained.'
      if [[ $code == 0 ]] && regular_file "$output/$relative/comparison.json"; then status=completed; reason=''; fi
    fi
    update_index "$output" '(.comparisons[]|select(.cohort==$c and .cpus==$n and .left==$l and .right==$r)) += {status:$status,reason:(if $reason=="" then null else $reason end),exit_code:$code}' \
      --arg c "$cohort" --argjson n "$cpus" --arg l "$left" --arg r "$right" --arg status "$status" --arg reason "$reason" --argjson code "$code"
    if [[ $status == completed ]]; then
      update_index "$output" '(.comparisons[]|select(.cohort==$c and .cpus==$n and .left==$l and .right==$r)) += {path:$path,sha256:$sha}' \
        --arg c "$cohort" --argjson n "$cpus" --arg l "$left" --arg r "$right" --arg path "$relative/comparison.json" --arg sha "$(sha256_file "$output/$relative/comparison.json")"
    fi
  done < "$output/control/assigned-comparisons.tsv"
}

# Keep this file sourceable for offline schedule/integrity fixtures.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then matrix_main "$@"; fi

