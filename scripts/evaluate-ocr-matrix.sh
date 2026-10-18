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

