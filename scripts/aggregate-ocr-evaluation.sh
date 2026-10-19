#!/usr/bin/env bash
# Read-only aggregation: no Docker execution, OCR, network access or repair.
# jq variables are intentionally passed literally through update_index.
# shellcheck disable=SC2016
# shellcheck source-path=SCRIPTDIR
set -Eeuo pipefail
SCRIPT_DIRECTORY=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
# shellcheck source=evaluate-ocr-matrix.sh
source "$SCRIPT_DIRECTORY/evaluate-ocr-matrix.sh"

verify_indexed_file() {
  local root=$1 path=$2 expected=$3
  safe_relative "$path" && [[ $expected =~ ^[a-f0-9]{64}$ ]] &&
    regular_file "$root/$path" && [[ $(sha256_file "$root/$path") == "$expected" ]]
}

validate_shard() {
  local root=$1 expected=$2 cpus=$3 repository=$4 source_sha=$5 release_id=$6 attempt=$7 scratch=$8
  local path digest variant budget complete raw manifest journal_sha
  regular_file "$root/study-index.json" && [[ $(stat -c %s -- "$root/study-index.json") -le 4194304 ]] || return 1
  verify_evidence_manifest "$root" || return 1
  jq -e --slurpfile expected "$expected" --argjson cpus "$cpus" --arg repository "$repository" \
    --arg sha "$source_sha" --arg run "$release_id" --arg attempt "$attempt" '
    .schema_version==1 and .source_sha==$sha and .provenance.repository==$repository and
    .provenance.release_run_id==$run and .provenance.release_run_attempt==$attempt and .assigned_cpu_budget==$cpus and
    (.assigned_budget_complete|type)=="boolean" and (.status=="measured" or .status=="partial") and
    .manifest_sha256==$expected[0].manifest_sha256 and .protocol_sha256==$expected[0].protocol_sha256 and .studies==$expected[0].studies and
    .provenance.schedule_sha256==$expected[0].provenance.schedule_sha256 and
    ([.runs[]|{sequence,cohort,variant,cpus}]==[$expected[0].runs[]|{sequence,cohort,variant,cpus}]) and
    ([.comparisons[]|{cohort,cpus,left,right}]==[$expected[0].comparisons[]|{cohort,cpus,left,right}]) and
    all(.runs[]; if .cpus!=$cpus then .status=="not_assigned" and .driver_exit==null and .score_exit==null and .scores==null and .performance==null and .attempts==null and .driver_result==null and .raw_run==null
      else true end)
  ' "$root/study-index.json" >/dev/null 2> "$scratch/shape.stderr" || return 1
  # Validate status and paired nullable identities separately to keep this
  # admission readable; jq never turns a missing field into an accepted path.
  jq -e --argjson cpus "$cpus" '
    . as $index | .provenance as $provenance |
    all(["candidates","bundle","evaluator","evaluate_script","schedule"][]; . as $key |
      ($provenance[$key]|type)=="string" and ($provenance[$key]|length)>0 and
      ($provenance[$key+"_sha256"]|type)=="string" and ($provenance[$key+"_sha256"]|test("^[a-f0-9]{64}$"))) and
    ((.provenance.images==null)==(.provenance.images_sha256==null)) and
    (if any(.runs[]; .driver_exit!=null) then .provenance.images!=null else true end) and
    all(.runs[]|select(.cpus==$cpus); .status as $status | ["planned","interrupted","failed","completed"]|index($status)!=null) and
    all(.runs[]; ((.scores==null)==(.sha256==null)) and ((.performance==null)==(.performance_sha256==null)) and
      ((.attempts==null)==(.attempts_sha256==null)) and ((.driver_result==null)==(.driver_result_sha256==null)) and
      (if .score_exit==0 then .scores!=null and .performance!=null and .attempts!=null else true end) and
      (if .status=="completed" then .driver_exit==0 and .score_exit==0 and .image_binding_exit==0 and .driver_result!=null and .raw_run!=null else true end)) and
    all(.comparisons[]; . as $comparison | ((.path==null)==(.sha256==null)) and
      (.status as $status | ["planned","not_assigned","interrupted","skipped","failed","completed"]|index($status)!=null) and
      (if .cpus!=$cpus then .status=="not_assigned" and .path==null else true end) and
      (if .status=="completed" then .exit_code==0 and .path!=null and
        ([$index.runs[]|select(.cohort==$comparison.cohort and .cpus==$comparison.cpus and
          (.variant==$comparison.left or .variant==$comparison.right))|.status]==["completed","completed"])
       else true end)) and
    (if .assigned_budget_complete then all(.runs[]|select(.cpus==$cpus); .status=="completed") else true end)
  ' "$root/study-index.json" >/dev/null || return 1
  jq -r '
    .runs[]|[.scores,.sha256],[.performance,.performance_sha256],[.attempts,.attempts_sha256],[.driver_result,.driver_result_sha256]|select(.[0]!=null)|@tsv
  ' "$root/study-index.json" > "$scratch/paths.tsv"
  {
    jq -r '.comparisons[]|select(.path!=null)|[.path,.sha256]|@tsv' "$root/study-index.json" || return 1
    jq -r '.studies[]|[.manifest,.manifest_sha256],[.protocol,.protocol_sha256]|@tsv' "$root/study-index.json" || return 1
    jq -r '.provenance|[.candidates,.candidates_sha256],[.bundle,.bundle_sha256],[.evaluator,.evaluator_sha256],[.evaluate_script,.evaluate_script_sha256],[.schedule,.schedule_sha256],[.images,.images_sha256]|select(.[0]!=null)|@tsv' "$root/study-index.json" || return 1
  } >> "$scratch/paths.tsv"
  while IFS=$'\t' read -r path digest; do verify_indexed_file "$root" "$path" "$digest" || return 1; done < "$scratch/paths.tsv"
  jq -r '.runs[]|select(.score_exit==0)|[.scores,.variant,.cpus,.status,.raw_run,.cohort]|@tsv' "$root/study-index.json" > "$scratch/scores.tsv"
  local cohort
  while IFS=$'\t' read -r path variant budget complete raw cohort; do
    safe_relative "$raw" || return 1
    manifest=$(jq -er --arg c "$cohort" '.studies[]|select(.cohort==$c)|.manifest_sha256' "$root/study-index.json")
    jq -e --arg variant "$variant" --argjson cpus "$budget" --arg manifest "$manifest" --arg complete "$complete" \
      '.variant==$variant and .cpus==$cpus and .manifest_sha256==$manifest and (if $complete=="completed" then .journal_complete==true else true end)' "$root/$path" >/dev/null || return 1
    journal_sha=$(jq -er '.journal_sha256' "$root/$path")
    verify_indexed_file "$root" "$raw/requests.jsonl" "$journal_sha" || return 1
  done < "$scratch/scores.tsv"
  jq -r '.runs[]|select(.status=="completed")|.driver_result' "$root/study-index.json" > "$scratch/completed.txt"
  while IFS= read -r path; do jq -e '.complete==true' "$root/$path" >/dev/null || return 1; done < "$scratch/completed.txt"
}

aggregate_main() {
  local one='' two='' output='' repository='' source_sha='' release_id='' attempt='' laboratory=true one_status=success two_status=success
  local repository_root cpus input download status reason root expected temporary agreed='' current source destination
  repository_root=$(cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)
  while (($#)); do
    (($#>=2)) || return 2
    case $1 in
      --cpu-one) one=$2;; --cpu-two) two=$2;; --output) output=$2;; --repository) repository=$2;;
      --source-sha) source_sha=$2;; --release-run-id) release_id=$2;; --release-run-attempt) attempt=$2;;
      --include-laboratory) laboratory=$2;; --cpu-one-download-status) one_status=$2;; --cpu-two-download-status) two_status=$2;;
      *) fail "Unknown aggregation option: $1"; return 2;;
    esac
    shift 2
  done
  [[ $source_sha =~ ^[a-f0-9]{40}$ && $repository =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ &&
    $release_id =~ ^[1-9][0-9]*$ && $attempt =~ ^[1-9][0-9]*$ && $laboratory =~ ^(true|false)$ &&
    $one_status =~ ^(success|failure|skipped|cancelled)$ && $two_status =~ ^(success|failure|skipped|cancelled)$ ]] || return 2
  [[ -n $output && ! -e $output && ! -L $output ]] || return 1
  mkdir -p -- "$output/control" "$output/provenance" "$output/shards"
  output=$(realpath -e -- "$output")
  build_schedule "$repository_root" "$output" "$laboratory" "$source_sha" "$release_id" "$attempt" all
  expected=$output/control/expected-index.json
  cp -- "$output/study-index.json" "$expected"
  # Copy only the frozen source identities needed by the combined index; the
  # admitted shard directories retain their full published source snapshots.
  jq -r '.studies[]|.manifest,.protocol' "$expected" > "$output/control/study-inputs.txt"
  while IFS= read -r destination; do
    safe_relative "$destination" || return 1
    source=$repository_root/${destination#provenance/source/}
    mkdir -p -- "${output}/${destination%/*}"
    cp -- "$source" "$output/$destination"
  done < "$output/control/study-inputs.txt"
  update_index "$output" '.provenance += {repository:$repository,shards:[]}' --arg repository "$repository"
  for cpus in 1 2; do
    input=$one; download=$one_status
    [[ $cpus == 1 ]] || { input=$two; download=$two_status; }
    status=missing; reason='The assigned budget artifact did not contain a study index.'
    root=$output/shards/cpu$cpus/ocr-evaluation-results
    temporary=$output/control/cpu$cpus
    mkdir -p -- "$temporary"
    if [[ $download != success ]]; then
      status=download_failed; reason='GitHub artifact download or digest validation failed; extracted files are not admitted.'
    elif [[ -d $input && ! -L $input && -f $input/ocr-evaluation-results/study-index.json ]]; then
      status=rejected; reason='Shard selection, file identity, completeness or provenance admission failed.'
      if [[ -z $(find "$input" ! -type d ! -type f -print -quit) ]]; then
        cp -a -- "$input" "$output/shards/cpu$cpus"
        if validate_shard "$root" "$expected" "$cpus" "$repository" "$source_sha" "$release_id" "$attempt" "$temporary"; then
          current=$(jq -c '.provenance|{candidates_sha256,bundle_sha256,evaluator_sha256,evaluate_script_sha256}' "$root/study-index.json")
          if [[ -z $agreed || $agreed == "$current" ]]; then
            agreed=$current; status=admitted; reason=''
          else
            reason='The two budget artifacts do not use identical candidate, bundle, evaluator and controller bytes.'
          fi
        fi
      fi
    fi
    if [[ $status == admitted ]]; then
      jq --slurpfile shard "$root/study-index.json" --argjson cpus "$cpus" --arg prefix "shards/cpu$cpus/ocr-evaluation-results/" \
        'def prefix_fields($keys): reduce $keys[] as $key (. ; if .[$key]!=null then .[$key]=$prefix+.[$key] else . end);
        .runs |= map(if .cpus==$cpus then . as $planned | $shard[0].runs[]|select(.sequence==$planned.sequence)|prefix_fields(["scores","performance","attempts","raw_run","driver_result","parent_before","parent_after","parent_counters_after"]) else . end) |
        .comparisons |= map(if .cpus==$cpus then . as $planned | $shard[0].comparisons[]|select(.cohort==$planned.cohort and .cpus==$planned.cpus and .left==$planned.left and .right==$planned.right)|prefix_fields(["path"]) else . end) |
        .provenance.shards += [{cpus:$cpus,status:"admitted",index:($prefix+"study-index.json"),assigned_budget_complete:$shard[0].assigned_budget_complete,provenance:$shard[0].provenance}]' \
        "$output/study-index.json" > "$output/study-index.next.json"
      mv -- "$output/study-index.next.json" "$output/study-index.json"
      update_index "$output" '(.provenance.shards[]|select(.cpus==$cpus)).index_sha256=$sha' --argjson cpus "$cpus" --arg sha "$(sha256_file "$root/study-index.json")"
    else
      update_index "$output" \
        '(.runs[]|select(.cpus==$cpus)) += {status:$status,reason:$reason}|(.comparisons[]|select(.cpus==$cpus)) += {status:"skipped",reason:$reason}|.provenance.shards += [{cpus:$cpus,status:$status,reason:$reason,assigned_budget_complete:false}]' \
        --argjson cpus "$cpus" --arg status "$status" --arg reason "$reason"
    fi
  done
  update_index "$output" \
    '.assigned_budget_complete=([.provenance.shards[].assigned_budget_complete]|all)|.status=(if .assigned_budget_complete and ([.runs[].status=="completed"]|all) and ([.comparisons[].status=="completed"]|all) then "measured" else "partial" end)|.provenance.aggregated_utc=$utc' \
    --arg utc "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  write_evidence_manifest "$output"
  [[ $(jq -er '.status' "$output/study-index.json") == measured ]]
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then aggregate_main "$@"; fi
