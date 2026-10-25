#!/usr/bin/env bash
# Independent numerical replay of one sealed combined measurement: admits the
# Signed release run that sealed it, re-acquires the hash-locked references
# the way the matrix does and runs the committed Python checker. No OCR,
# scoring, Docker or repair; a nonzero checker result fails this command.
# jq programs and their variables are intentionally literal.
# shellcheck disable=SC2016
# shellcheck source-path=SCRIPTDIR
set -Eeuo pipefail
SCRIPT_DIRECTORY=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
# shellcheck source=evaluate-ocr-matrix.sh
source "$SCRIPT_DIRECTORY/evaluate-ocr-matrix.sh"
# Finite checker budget; replay.py never subsamples bootstrap draws to fit it.
REPLAY_WALL_SECONDS=2700
REPLAY_CPU_SECONDS=1800
REPLAY_ADDRESS_SPACE_BYTES=4294967296

# The newest completed Signed release run of this repository whose combined
# evidence is still retained; prints that artifact's name.
newest_sealed_evidence() {
  local repository=$1 run sha name
  while IFS=$'\t' read -r run sha; do
    [[ $run =~ ^[1-9][0-9]*$ && $sha =~ ^[a-f0-9]{40}$ ]] || return 1
    name=$(gh api "repos/$repository/actions/runs/$run/artifacts?per_page=100" | jq -r --arg prefix "ocr-evaluation-$sha-$run-" '
      [.artifacts[]|select(.expired==false and (.name|startswith($prefix)) and (.name|endswith("-combined")))|.name|
        select(ltrimstr($prefix)|rtrimstr("-combined")|test("^[1-9][0-9]*$"))]|
      sort_by(ltrimstr($prefix)|rtrimstr("-combined")|tonumber)|last // empty') || return 1
    if [[ -n $name ]]; then printf '%s\n' "$name"; return 0; fi
  done < <(gh api "repos/$repository/actions/workflows/release.yml/runs?status=completed&per_page=30" |
    jq -r --arg repository "$repository" '.workflow_runs[]|select(.head_repository.full_name==$repository)|[.id,.head_sha]|@tsv')
  return 1
}

# Names the evidence (or the newest one) and admits its run attempt; prints
# GitHub step outputs.
select_evidence() {
  local repository=$1 name=$2 record=$3 identity sha run attempt
  if [[ -z $name ]]; then
    name=$(newest_sealed_evidence "$repository") || { fail 'No retained combined evidence of a Signed release run'; return 1; }
  fi
  identity=$(sealed_evidence_identity "$name" "$repository" "$record") || { fail "Not sealed by a Signed release run of $repository: $name"; return 1; }
  read -r sha run attempt <<< "$identity"
  printf 'name=%s\nrun=%s\nattempt=%s\nsource_sha=%s\n' "$name" "$run" "$attempt" "$sha"
}

# The evaluator bytes every admitted budget shard executed, as
# "<relative path>\t<sha256>"; the aggregate admitted only identical bytes.
evidence_evaluator() {
  jq -er '[.provenance.shards[]|select(.status=="admitted")] as $shards |
    if ($shards|length)>0 and ([$shards[].provenance.evaluator_sha256]|unique|length)==1 and
       all($shards[]; (.index|endswith("/study-index.json")) and (.provenance.evaluator|type)=="string")
    then $shards[0]|"\(.index|rtrimstr("study-index.json"))\(.provenance.evaluator)\t\(.provenance.evaluator_sha256)"
    else error("no single admitted evaluator") end' "$1/study-index.json"
}

replay_main() {
  local evidence='' source_sha='' python='' evaluator='' work='' output='' repository path digest code=0
  repository=$(cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)
  while (($#)); do
    (($#>=2)) || { fail 'Every replay option requires a value'; return 2; }
    case $1 in
      --evidence) evidence=$2;; --source-sha) source_sha=$2;; --python) python=$2;;
      --evaluator) evaluator=$2;; --work) work=$2;; --output) output=$2;;
      *) fail "Unknown replay option: $1"; return 2;;
    esac
    shift 2
  done
  [[ $source_sha =~ ^[a-f0-9]{40}$ && -d $evidence && ! -L $evidence && -x $python ]] || { fail 'Invalid replay evidence, source or Python'; return 2; }
  [[ -n $work && -n $output && ! -e $work && ! -e $output && ! -L $work && ! -L $output ]] || return 1
  for path in jq git tar sha256sum timeout prlimit; do command -v "$path" >/dev/null || return 1; done
  evidence=$(realpath -e -- "$evidence")
  verify_evidence_manifest "$evidence" || { fail 'The combined evidence fails its own SHA256SUMS'; return 1; }
  jq -e --arg sha "$source_sha" '.schema_version==1 and .source_sha==$sha and .assigned_cpu_budget==null' \
    "$evidence/study-index.json" >/dev/null || { fail 'The combined index names another source or one budget'; return 1; }
  mkdir -p -- "$work" "$output/control" "$output/provenance/source"
  work=$(realpath -e -- "$work"); output=$(realpath -e -- "$output")
  # By default acquisition runs the evaluator the measurement itself ran.
  if [[ -z $evaluator ]]; then
    IFS=$'\t' read -r path digest <<< "$(evidence_evaluator "$evidence")"
    safe_relative "$path" && regular_file "$evidence/$path" && [[ $(sha256_file "$evidence/$path") == "$digest" ]] ||
      { fail 'The admitted evaluator bytes are absent or changed'; return 1; }
    evaluator=$evidence/$path
  fi
  regular_file "$evaluator" || return 1
  cp -- "$evaluator" "$output/provenance/indra-evaluate"
  chmod 0755 "$output/provenance/indra-evaluate"
  # The frozen locks and committed fixtures of the measured source, from Git.
  git -C "$repository" archive --format=tar "$source_sha" evaluation/acquisition evaluation/corpus-frozen.json \
    evaluation/protocol.json evaluation/fixtures evaluation/laboratory-supplement > "$output/provenance/source.tar"
  tar -xf "$output/provenance/source.tar" -C "$output/provenance/source"
  acquire_reference_inputs "$output" "$work" "$output/provenance/indra-evaluate" true
  capture "$output" replay "$REPLAY_WALL_SECONDS" prlimit --cpu="$REPLAY_CPU_SECONDS" --as="$REPLAY_ADDRESS_SPACE_BYTES" -- \
    "$python" -I -B "$repository/evaluation/independent/replay.py" --repository "$repository" --source-sha "$source_sha" \
    --main-references "$work/main" --laboratory-references "$work/laboratory" --evidence "$evidence" \
    --output "$output/replay.json" || code=$?
  write_evidence_manifest "$output"
  if regular_file "$output/replay.json"; then
    jq -r --arg sha "$(sha256_file "$output/replay.json")" --arg code "$code" '
      "| Replay | Value |", "| --- | --- |", "| passed | \(.passed) |", "| checker exit | \($code) |",
      "| checked scalar fields | \(.checked_scalar_fields) |", "| arithmetic differences | \(.arithmetic_differences|length) |",
      "| structural failures | \(.failures|length) |", "| receipt SHA-256 | `\($sha)` |"' "$output/replay.json"
  fi
  return "$code"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  case ${1:-} in
    select) shift; [[ $# == 3 ]] || { fail 'usage: replay-ocr-evaluation.sh select REPOSITORY NAME RECORD'; exit 2; }; select_evidence "$@";;
    run) shift; replay_main "$@";;
    *) fail 'usage: replay-ocr-evaluation.sh select REPOSITORY NAME RECORD | run --evidence DIR --source-sha SHA --python PATH --work NEW --output NEW [--evaluator PATH]'; exit 2;;
  esac
fi
