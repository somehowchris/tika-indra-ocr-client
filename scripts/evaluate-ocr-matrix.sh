#!/usr/bin/env bash
# RECOVERY WIP: helper functions only; the execution entry point is not yet restored.
# This checkpoint must not be used to claim a completed scientific matrix.
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
    # --no-clobber also protects a destination created after the check above.
    cp --no-clobber -- "$source" "$root/$relative" || return 1
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

write_evidence_manifest() {
  local root=$1 temporary listing relative
  temporary=$(mktemp "${root%/*}/.evidence-manifest.XXXXXXXX") || return 1
  listing=$(mktemp "${root%/*}/.evidence-list.XXXXXXXX") || { rm -f -- "$temporary"; return 1; }
  if ! (cd "$root" && find . -type f ! -path './SHA256SUMS' ! -path './SHA256SUMS.partial' -print | sort) > "$listing"; then
    rm -f -- "$temporary" "$listing"; return 1
  fi
  while IFS= read -r relative; do
    relative=${relative#./}
    safe_relative "$relative" || { rm -f -- "$temporary" "$listing"; return 1; }
    printf '%s  %s\n' "$(sha256_file "$root/$relative")" "$relative" >> "$temporary"
  done < "$listing"
  rm -f -- "$listing"
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
  expected=$(find "$root" -type f ! -path "$root/SHA256SUMS" ! -path "$root/SHA256SUMS.partial" -printf '.\n' | wc -l)
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
      runs:[$schedule[0][]|.+{status:(if $budget=="all" or .cpus==($budget|tonumber) then "planned" else "not_assigned" end),driver_exit:null,score_exit:null,reason:null,scores:null,sha256:null,performance:null,performance_sha256:null,attempts:null,attempts_sha256:null,raw_run:null,driver_result:null,driver_result_sha256:null,cgroup:null,parent_before:null,parent_after:null,parent_counters_after:null}],
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
    scripts/aggregate-ocr-evaluation.sh scripts/test-evaluate-ocr-matrix.sh \
    .github/workflows/ocr-evaluation.yml > "$output/provenance/source.tar"
  tar -xf "$output/provenance/source.tar" -C "$output/provenance/source"
}

record_host() {
  local output=$1 allowed first last part cpu
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
  jq -n --arg allowed "$allowed" --arg one "$MATRIX_CPU_ONE" --arg two "$MATRIX_CPU_TWO" \
    '{process_allowed_cpus:$allowed,selected_cpu_one:$one,selected_cpu_two:$two,scope:"Actual selected logical CPUs; 1/2-budget jobs may have unlike hosts. Do not interpret cross-host differences as isolated CPU scaling."}' > "$output/provenance/affinity.json"
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
  local evaluator='' candidates='' bundle='' repository='' source_sha='' release_id='' attempt='' work='' output='' laboratory=true budget=all
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
      *) fail "Unknown matrix option: $1"; return 2;;
    esac
    shift 2
  done
  [[ $source_sha =~ ^[a-f0-9]{40}$ && $repository =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ &&
     $release_id =~ ^[1-9][0-9]*$ && $attempt =~ ^[1-9][0-9]*$ &&
     $laboratory =~ ^(true|false)$ && $budget =~ ^(1|2|all)$ ]] || { fail 'Invalid matrix identity or budget'; return 2; }
  for name in bash jq sha256sum timeout git tar docker systemctl sudo gh cosign curl; do command -v "$name" >/dev/null || return 1; done
  regular_file "$evaluator" && [[ -x $evaluator ]] && regular_file "$candidates" && regular_file "$bundle" || return 1
  [[ -n $work && -n $output && ! -e $work && ! -e $output && ! -L $work && ! -L $output ]] || return 1
  mkdir -p -- "$work" "$output/provenance" "$output/control"
  work=$(realpath -e -- "$work"); output=$(realpath -e -- "$output")
  MATRIX_OUTPUT=$output
  MATRIX_ACTIVE_SLICE=''; MATRIX_ACTIVE_UNIT=''
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
  record_host "$output"
  acquire_inputs "$output" "$work" "$evaluator" "$laboratory"
  execute_schedule "$output" "$work" "$evaluator" "$budget"
  execute_comparisons "$output" "$evaluator" "$budget"
  [[ $(jq '[.runs[]|select(.status!="not_assigned")|.status=="completed"]|all' "$output/study-index.json") == true ]]
}

# Keep this file sourceable for offline schedule/integrity fixtures.
if [[ ${BASH_SOURCE[0]} == "$0" ]]; then matrix_main "$@"; fi

