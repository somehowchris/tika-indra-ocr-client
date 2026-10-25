#!/usr/bin/env bash
# Offline integrity fixtures only: never Docker, network, systemd or OCR.
# jq expressions passed to update_index are intentionally literal.
# shellcheck disable=SC2016
# shellcheck source-path=SCRIPTDIR
set -Eeuo pipefail
SCRIPT_DIRECTORY=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
REPOSITORY_ROOT=$(cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)
# shellcheck source=evaluate-ocr-matrix.sh
source "$SCRIPT_DIRECTORY/evaluate-ocr-matrix.sh"
TEST_PARENT=$REPOSITORY_ROOT/evaluation/runner/target
mkdir -p -- "$TEST_PARENT"
TEST_ROOT=$(mktemp -d "$TEST_PARENT/matrix-fixture.XXXXXXXX")
TEST_COMPLETE=false
finish_fixture() {
  if [[ $TEST_COMPLETE == true && $TEST_ROOT == "$TEST_PARENT"/matrix-fixture.* ]]; then
    rm -rf -- "$TEST_ROOT"
  else
    printf 'Retained failed offline fixture: %s\n' "$TEST_ROOT" >&2
  fi
}
trap finish_fixture EXIT

expect_failure() {
  if "$@"; then fail 'Expected rejection did not occur'; return 1; fi
}

mkdir -p "$TEST_ROOT/seed"
printf 'fixed bytes\n' > "$TEST_ROOT/source.txt"
EXPECTED_HASH=$(sha256_file "$TEST_ROOT/source.txt")
seed_file "$TEST_ROOT/source.txt" "$TEST_ROOT/seed" a/b.txt "$EXPECTED_HASH" 12
seed_file "$TEST_ROOT/source.txt" "$TEST_ROOT/seed" a/b.txt "$EXPECTED_HASH" 12
printf changed > "$TEST_ROOT/seed/a/b.txt"
expect_failure seed_file "$TEST_ROOT/source.txt" "$TEST_ROOT/seed" a/b.txt "$EXPECTED_HASH" 12
ln -s absent "$TEST_ROOT/seed/dangling"
expect_failure seed_file "$TEST_ROOT/source.txt" "$TEST_ROOT/seed" dangling "$EXPECTED_HASH" 12
ln -s a "$TEST_ROOT/seed/parent-link"
expect_failure seed_file "$TEST_ROOT/source.txt" "$TEST_ROOT/seed" parent-link/other.txt "$EXPECTED_HASH" 12
expect_failure safe_relative ../outside
expect_failure safe_relative /absolute
expect_failure safe_relative a/./b

mkdir -p "$TEST_ROOT/evidence/nested"
printf outer > "$TEST_ROOT/evidence/value.txt"
printf inner > "$TEST_ROOT/evidence/nested/SHA256SUMS"
printf temporary > "$TEST_ROOT/evidence/nested/SHA256SUMS.partial"
# The native server's fetch lock stays in the model revision directory, zero
# bytes and unreadable to the controller (release run 39766403121 failed here).
mkdir -p "$TEST_ROOT/evidence/models/bd02167fdf2005a19962cc15188f8496a2c87d3b"
: > "$TEST_ROOT/evidence/models/bd02167fdf2005a19962cc15188f8496a2c87d3b/.fetch.lock"
chmod 000 "$TEST_ROOT/evidence/models/bd02167fdf2005a19962cc15188f8496a2c87d3b/.fetch.lock"
write_evidence_manifest "$TEST_ROOT/evidence"
verify_evidence_manifest "$TEST_ROOT/evidence"
[[ $(wc -l < "$TEST_ROOT/evidence/SHA256SUMS") == 3 ]]
expect_failure grep -q 'fetch.lock' "$TEST_ROOT/evidence/SHA256SUMS"
printf changed >> "$TEST_ROOT/evidence/nested/SHA256SUMS"
expect_failure verify_evidence_manifest "$TEST_ROOT/evidence"

literal='a $literal $(not-a-command) `also-literal` [x]'
capture "$TEST_ROOT" literal 5 printf '%s' "$literal"
[[ $(cat "$TEST_ROOT/control/literal.stdout") == "$literal" ]]
jq -e --arg value "$literal" '.[2]==$value' "$TEST_ROOT/control/literal.argv.json" >/dev/null
capture_code=0
capture "$TEST_ROOT" nonzero 5 bash -c 'exit 7' || capture_code=$?
[[ $capture_code == 7 ]]
jq -e '.exit_code==7 and .timed_out==false' "$TEST_ROOT/control/nonzero.status.json" >/dev/null
capture_code=0
capture "$TEST_ROOT" deadline 1 bash -c 'sleep 3' || capture_code=$?
[[ $capture_code == 124 ]]
jq -e '.exit_code==124 and .timed_out==true' "$TEST_ROOT/control/deadline.status.json" >/dev/null

# A tiny fake daemon exercises ownership and confirmation paths. No real
# Docker command is available through this fixture's overridden PATH.
mkdir -p "$TEST_ROOT/mock-bin"
cat > "$TEST_ROOT/mock-bin/docker" <<'MOCK_DOCKER'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >> "$MOCK_CONTAINER_ROOT/calls.txt"
case "${1:-} ${2:-}" in
  'context show') printf 'mock-context\n';;
  'context inspect') [[ ${*: -1} == mock-context ]] || exit 1; printf 'unix:///tmp/mock-daemon/docker.sock\n';;
  'container ls')
    [[ $(cat "$MOCK_CONTAINER_ROOT/state") == absent ]] || printf '%s\n' "$MOCK_CONTAINER_ID";;
  'container inspect')
    token=$MATRIX_MODEL_TOKEN
    [[ ${MOCK_WRONG_OWNER:-false} == false ]] || token=foreign
    running=false
    [[ $(cat "$MOCK_CONTAINER_ROOT/state") != running ]] || running=true
    jq -n --arg id "$MOCK_CONTAINER_ID" --arg name "/$MATRIX_MODEL_CONTAINER" --arg token "$token" \
      --arg image "$MATRIX_MODEL_IMAGE" --argjson running "$running" \
      '[{Id:$id,Name:$name,Image:$image,Config:{Labels:{"io.indra.evaluation.models":$token}},State:{Running:$running}}]';;
  'container stop')
    [[ ${MOCK_STOP_FAIL:-false} == false ]] || exit 7
    printf stopped > "$MOCK_CONTAINER_ROOT/state";;
  'container rm')
    [[ $(cat "$MOCK_CONTAINER_ROOT/state") == stopped ]] || exit 9
    printf absent > "$MOCK_CONTAINER_ROOT/state";;
  *) exit 99;;
esac
MOCK_DOCKER
chmod u+x "$TEST_ROOT/mock-bin/docker"
model_cleanup_fixture() (
  local name=$1 state=$2 wrong_owner=$3 stop_fail=$4 expected=$5 code=0 call
  export PATH="$TEST_ROOT/mock-bin:$PATH" MOCK_CONTAINER_ROOT="$TEST_ROOT/model-$name" MOCK_WRONG_OWNER="$wrong_owner" MOCK_STOP_FAIL="$stop_fail"
  export MOCK_CONTAINER_ID=3333333333333333333333333333333333333333333333333333333333333333
  export MATRIX_MODEL_TOKEN=11111111111111111111111111111111
  export MATRIX_MODEL_CONTAINER=indra-models-12345-fetch-$MATRIX_MODEL_TOKEN
  export MATRIX_MODEL_IMAGE=sha256:2222222222222222222222222222222222222222222222222222222222222222
  MATRIX_MODEL_PHASE=fetch; MATRIX_MODEL_CLEANUP_ATTEMPT=0
  mkdir -p "$MOCK_CONTAINER_ROOT"
  printf '%s' "$state" > "$MOCK_CONTAINER_ROOT/state"
  cleanup_model_container "$MOCK_CONTAINER_ROOT" || code=$?
  if [[ $expected == cleaned ]]; then
    [[ $code == 0 && -z $MATRIX_MODEL_CONTAINER && $(cat "$MOCK_CONTAINER_ROOT/state") == absent ]]
  else
    [[ $code != 0 && -n $MATRIX_MODEL_CONTAINER && $(cat "$MOCK_CONTAINER_ROOT/state") == "$state" ]]
    while IFS= read -r call; do [[ $call != 'container rm '* ]] || return 1; done < "$MOCK_CONTAINER_ROOT/calls.txt"
  fi
)
# The evaluator must be pointed at the daemon the controller used: the active
# context's endpoint, or an explicit DOCKER_HOST; never a TCP endpoint.
( export PATH="$TEST_ROOT/mock-bin:$PATH" MOCK_CONTAINER_ROOT="$TEST_ROOT/model-host"; mkdir -p "$MOCK_CONTAINER_ROOT"
  unset DOCKER_HOST
  [[ $(matrix_docker_host) == unix:///tmp/mock-daemon/docker.sock ]]
  [[ $(DOCKER_HOST=unix:///run/other/docker.sock matrix_docker_host) == unix:///run/other/docker.sock ]]
  # Call the sourced function itself: a child shell sourcing the controller
  # with $0 set to its path would run matrix_main and fail for another reason.
  DOCKER_HOST=tcp://127.0.0.1:2375 expect_failure matrix_docker_host )
# Arms are bounded by the controller's remaining wall budget, so the inner
# timeout precedes the job's outer one and this controller seals the evidence.
( MATRIX_DEADLINE=''; [[ $(cell_wall_cap 17320 1000) == 17320 ]]
  MATRIX_DEADLINE=$((1000 + 17320 + MATRIX_CELL_RESERVE_SECONDS)); [[ $(cell_wall_cap 17320 1000) == 17320 ]]
  MATRIX_DEADLINE=$((1000 + 5000 + MATRIX_CELL_RESERVE_SECONDS)); [[ $(cell_wall_cap 17320 1000) == 5000 ]]
  MATRIX_DEADLINE=$((1000 + MATRIX_MINIMUM_CELL_SECONDS + MATRIX_CELL_RESERVE_SECONDS)); [[ $(cell_wall_cap 17320 1000) == "$MATRIX_MINIMUM_CELL_SECONDS" ]]
  MATRIX_DEADLINE=$((1000 + MATRIX_MINIMUM_CELL_SECONDS - 1 + MATRIX_CELL_RESERVE_SECONDS)); expect_failure cell_wall_cap 17320 1000
  MATRIX_DEADLINE=''; expect_failure cell_wall_cap 0 1000; expect_failure cell_wall_cap 17320 later )
# The systemd backend runs the driver as root through sudo; its evidence is
# reclaimed for this user before binding, scoring, the manifest and upload.
cat > "$TEST_ROOT/mock-bin/sudo" <<'MOCK_SUDO'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${MOCK_SUDO_CALLS:?}"
exec "$@"
MOCK_SUDO
chmod u+x "$TEST_ROOT/mock-bin/sudo"
( export PATH="$TEST_ROOT/mock-bin:$PATH" MOCK_SUDO_CALLS="$TEST_ROOT/reclaim/sudo-calls.txt"
  driver=$TEST_ROOT/reclaim/runs/main/cpu1/arm/driver
  mkdir -p "$driver/run" "$TEST_ROOT/reclaim/control"
  printf '{}\n' > "$driver/run/requests.jsonl"
  MATRIX_RESOURCE_BACKEND=systemd reclaim_driver_evidence "$TEST_ROOT/reclaim" 7 "$driver"
  jq -e '.exit_code==0' "$TEST_ROOT/reclaim/control/cell-7-driver-ownership.status.json" >/dev/null
  jq -e --arg owner "$(id -u):$(id -g)" --arg driver "$driver" '.==["sudo","chown","-R",$owner,$driver]' \
    "$TEST_ROOT/reclaim/control/cell-7-driver-ownership.argv.json" >/dev/null
  [[ $(wc -l < "$MOCK_SUDO_CALLS") -eq 1 ]]
  # The Docker backend runs the driver as this user, and a driver that never
  # created its directory leaves nothing to reclaim.
  MATRIX_RESOURCE_BACKEND=docker reclaim_driver_evidence "$TEST_ROOT/reclaim" 8 "$driver"
  MATRIX_RESOURCE_BACKEND=systemd reclaim_driver_evidence "$TEST_ROOT/reclaim" 9 "$TEST_ROOT/reclaim/absent"
  [[ ! -e $TEST_ROOT/reclaim/control/cell-8-driver-ownership.argv.json && ! -e $TEST_ROOT/reclaim/control/cell-9-driver-ownership.argv.json ]]
  [[ $(wc -l < "$MOCK_SUDO_CALLS") -eq 1 ]] )
model_cleanup_fixture running running false false cleaned
model_cleanup_fixture stopped stopped false false cleaned
model_cleanup_fixture absent absent false false cleaned
model_cleanup_fixture foreign running true false retained
model_cleanup_fixture unconfirmed running false true retained

SOURCE_SHA=$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)
build_schedule "$REPOSITORY_ROOT" "$TEST_ROOT/planned" true "$SOURCE_SHA" 12345 1 all
jq -e '(.runs|length)==24 and (.comparisons|length)==60 and .studies[0].cohort=="main" and .studies[1].cohort=="laboratory" and .studies[1].bootstrap_resamples==0 and .studies[0].bootstrap_resamples==10000 and
  ([.runs[]|select(.cpus==1)|{cohort,variant}]==[.runs[]|select(.cpus==2)|{cohort,variant}])' "$TEST_ROOT/planned/study-index.json" >/dev/null
build_schedule "$REPOSITORY_ROOT" "$TEST_ROOT/no-lab" false "$SOURCE_SHA" 12345 1 1
jq -e '(.runs|length)==12 and (.comparisons|length)==30 and all(.runs[]|select(.cpus==2); .status=="not_assigned")' "$TEST_ROOT/no-lab/study-index.json" >/dev/null

LAB=$REPOSITORY_ROOT/evaluation/laboratory-supplement
jq -e --slurpfile original "$LAB/acquisition/sources.json" --slurpfile audit "$LAB/audit/metadata-recovery-0002.json" '
  (.assets|length)==4 and all(.assets[]; . as $asset|any($original[0].assets[]; .==$asset)) and
  ([.assets[]|select(.path|endswith(".pdf"))]==[$original[0].assets[]|select(.path|endswith(".pdf"))]) and
  ($audit[0].omitted_frozen_metadata|length)==2 and $audit[0].ocr_outputs_used==false
' "$LAB/acquisition/execution-sources.json" >/dev/null
jq -r '.recovered_assets[]|[.fixture,.sha256,.bytes]|@tsv' "$LAB/audit/metadata-recovery-0002.json" > "$TEST_ROOT/recovered.tsv"
while IFS=$'\t' read -r path digest bytes; do verify_file "$LAB/$path" "$digest" "$bytes"; done < "$TEST_ROOT/recovered.tsv"

# Prospective amendments must describe the protocol bytes actually in the tree:
# the always-math record's amended identities and the Indra 1.1 record's
# active identities are the same three files, unchanged by the 1.1 binding.
AUDITS=$REPOSITORY_ROOT/evaluation/audits
for protocol in evaluation/protocol.json evaluation/laboratory-supplement/protocol.json runtime/validation/telemetry-http-protocol.json; do
  digest=$(sha256_file "$REPOSITORY_ROOT/$protocol"); bytes=$(stat -c %s -- "$REPOSITORY_ROOT/$protocol")
  jq -e --arg p "$protocol" --arg digest "$digest" --argjson bytes "$bytes" \
    '.amended_protocols[$p]=={bytes:$bytes,sha256:$digest}' "$AUDITS/always-math-20261019/amendment.json" >/dev/null
  jq -e --arg p "$protocol" --arg digest "$digest" --argjson bytes "$bytes" \
    '.protocol_identities.active[$p]=={bytes:$bytes,sha256:$digest}' "$AUDITS/indra-1-1-20261020/amendment.json" >/dev/null
done
jq -e '.model_binding.after.revision=="bd02167fdf2005a19962cc15188f8496a2c87d3b" and .model_binding.after.package.version=="1.1.2" and
  .arm_settings.applies_to==["python-math-b64","rust-math-b64"] and .arm_settings.math==true and .arm_settings.lm_beam_search.enabled==true and
  .arm_settings.dual_read_routing.enabled==true and (.model_binding.after.files|length)==19 and
  all(.model_binding.after.files[]; (.sha256==null and .name=="math_meta.json") or (.sha256|test("^[a-f0-9]{64}$")))' \
  "$AUDITS/indra-1-1-20261020/amendment.json" >/dev/null

make_shard() {
  local cpus=$1 root=$TEST_ROOT/cpu$1/ocr-evaluation-results path key hash_key
  build_schedule "$REPOSITORY_ROOT" "$root" true "$SOURCE_SHA" 12345 1 "$cpus"
  jq -r '.studies[]|.manifest,.protocol' "$root/study-index.json" > "$TEST_ROOT/inputs-$cpus.txt"
  while IFS= read -r path; do
    mkdir -p -- "$root/${path%/*}"
    cp -- "$REPOSITORY_ROOT/${path#provenance/source/}" "$root/$path"
  done < "$TEST_ROOT/inputs-$cpus.txt"
  update_index "$root" '.provenance.repository="somehowchris/tika-indra-ocr-client"|(.runs[]|select(.cpus==$cpus)) += {status:"failed",reason:"Synthetic controller admission failure; no OCR occurred."}' --argjson cpus "$cpus"
  for key in candidates bundle evaluator evaluate_script; do
    printf 'SYNTHETIC %s; NOT A RELEASE OR OCR RESULT\n' "$key" > "$root/provenance/$key.fixture"
    hash_key=${key}_sha256
    update_index "$root" '.provenance += {($key):$path,($hash_key):$sha}' \
      --arg key "$key" --arg path "provenance/$key.fixture" --arg hash_key "$hash_key" --arg sha "$(sha256_file "$root/provenance/$key.fixture")"
  done
  write_evidence_manifest "$root"
}
make_shard 1
make_shard 2

aggregate_fixture() {
  local destination=$1 one=$2 two=$3 one_status=${4:-success} two_status=${5:-success} code=0
  bash "$SCRIPT_DIRECTORY/aggregate-ocr-evaluation.sh" --cpu-one "$one" --cpu-two "$two" \
    --cpu-one-download-status "$one_status" --cpu-two-download-status "$two_status" \
    --output "$TEST_ROOT/$destination" --repository somehowchris/tika-indra-ocr-client --source-sha "$SOURCE_SHA" \
    --release-run-id 12345 --release-run-attempt 1 --include-laboratory true || code=$?
  [[ $code == 1 ]]
  verify_evidence_manifest "$TEST_ROOT/$destination"
  jq -e '.status=="partial" and (.runs|length)==24 and (.comparisons|length)==60' "$TEST_ROOT/$destination/study-index.json" >/dev/null
}
aggregate_fixture paired-partial "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e 'all(.provenance.shards[]; .status=="admitted")' "$TEST_ROOT/paired-partial/study-index.json" >/dev/null
aggregate_fixture missing "$TEST_ROOT/cpu1" "$TEST_ROOT/absent"
jq -e '.provenance.shards[1].status=="missing" and all(.runs[]|select(.cpus==2); .scores==null and .status=="missing")' "$TEST_ROOT/missing/study-index.json" >/dev/null
aggregate_fixture invalid-download "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2" success failure
jq -e '.provenance.shards[1].status=="download_failed"' "$TEST_ROOT/invalid-download/study-index.json" >/dev/null

# Synthetic completed rows exercise path rewriting and comparison admission;
# their files explicitly carry a synthetic marker and are deleted on success.
make_completed_cell() {
  local variant=$1 root=$TEST_ROOT/cpu1/ocr-evaluation-results relative raw scored field hash_field file
  relative=runs/main/cpu1/$variant
  raw=$relative/driver/run; scored=$relative/scored
  mkdir -p "$root/$raw" "$root/$scored"
  printf '{"synthetic":true,"not_an_ocr_result":true}\n' > "$root/$raw/requests.jsonl"
  jq -n --arg variant "$variant" --arg manifest "$(sha256_file "$REPOSITORY_ROOT/evaluation/corpus-frozen.json")" \
    --arg journal "$(sha256_file "$root/$raw/requests.jsonl")" \
    '{synthetic:true,variant:$variant,cpus:1,manifest_sha256:$manifest,journal_sha256:$journal,journal_complete:true}' > "$root/$scored/scores.json"
  printf '{"synthetic":true}\n' > "$root/$scored/performance.json"
  printf 'synthetic,not_an_ocr_result\n' > "$root/$scored/attempts.csv"
  printf '{"synthetic":true,"complete":true}\n' > "$root/$relative/driver/driver-result.json"
  update_index "$root" '(.runs[]|select(.cpus==1 and .cohort=="main" and .variant==$variant)) += {status:"completed",driver_exit:0,score_exit:0,image_binding_exit:0,reason:null,raw_run:$raw}' \
    --arg variant "$variant" --arg raw "$raw"
  for field in scores performance attempts driver_result; do
    hash_field=${field}_sha256
    case $field in
      scores) file=$scored/scores.json; hash_field=sha256;;
      performance) file=$scored/performance.json;;
      attempts) file=$scored/attempts.csv;;
      driver_result) file=$relative/driver/driver-result.json;;
    esac
    update_index "$root" '(.runs[]|select(.cpus==1 and .cohort=="main" and .variant==$variant)) += {($field):$path,($hash_field):$sha}' \
      --arg variant "$variant" --arg field "$field" --arg path "$file" --arg hash_field "$hash_field" --arg sha "$(sha256_file "$root/$file")"
  done
}
LEFT_VARIANT=$(jq -er '.primary_variants[0].id' "$REPOSITORY_ROOT/evaluation/protocol.json")
RIGHT_VARIANT=$(jq -er '.primary_variants[1].id' "$REPOSITORY_ROOT/evaluation/protocol.json")
make_completed_cell "$LEFT_VARIANT"
make_completed_cell "$RIGHT_VARIANT"
SHARD_ONE=$TEST_ROOT/cpu1/ocr-evaluation-results
printf '{"synthetic":true,"not_a_daemon_identity":true}\n' > "$SHARD_ONE/provenance/images.fixture"
update_index "$SHARD_ONE" '.provenance.images="provenance/images.fixture"|.provenance.images_sha256=$sha' --arg sha "$(sha256_file "$SHARD_ONE/provenance/images.fixture")"
mkdir -p "$SHARD_ONE/comparisons/main/cpu1"
printf '{"synthetic":true,"not_an_ocr_result":true}\n' > "$SHARD_ONE/comparisons/main/cpu1/pair.json"
update_index "$SHARD_ONE" '(.comparisons[]|select(.cpus==1 and .cohort=="main" and .left==$left and .right==$right)) += {status:"completed",exit_code:0,path:"comparisons/main/cpu1/pair.json",sha256:$sha}' \
  --arg left "$LEFT_VARIANT" --arg right "$RIGHT_VARIANT" --arg sha "$(sha256_file "$SHARD_ONE/comparisons/main/cpu1/pair.json")"
write_evidence_manifest "$SHARD_ONE"
aggregate_fixture completed-paths "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e '([.runs[]|select(.status=="completed")]|length)==2 and all(.runs[]|select(.status=="completed");
  (.scores|startswith("shards/cpu1/ocr-evaluation-results/runs/")) and (.raw_run|startswith("shards/cpu1/ocr-evaluation-results/runs/"))) and
  all(.comparisons[]|select(.status=="completed"); .path=="shards/cpu1/ocr-evaluation-results/comparisons/main/cpu1/pair.json")' "$TEST_ROOT/completed-paths/study-index.json" >/dev/null
cp -- "$SHARD_ONE/study-index.json" "$TEST_ROOT/completed-index.json"
update_index "$SHARD_ONE" '(.runs[]|select(.cpus==1 and .cohort=="main" and .variant==$variant)).status="failed"' --arg variant "$LEFT_VARIANT"
write_evidence_manifest "$SHARD_ONE"
aggregate_fixture unmatched-comparison "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e '.provenance.shards[0].status=="rejected"' "$TEST_ROOT/unmatched-comparison/study-index.json" >/dev/null
cp -- "$TEST_ROOT/completed-index.json" "$SHARD_ONE/study-index.json"
update_index "$SHARD_ONE" 'del(.provenance.candidates,.provenance.candidates_sha256)'
write_evidence_manifest "$SHARD_ONE"
aggregate_fixture absent-provenance "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e '.provenance.shards[0].status=="rejected"' "$TEST_ROOT/absent-provenance/study-index.json" >/dev/null
cp -- "$TEST_ROOT/completed-index.json" "$SHARD_ONE/study-index.json"
write_evidence_manifest "$SHARD_ONE"

printf '\n' >> "$TEST_ROOT/cpu2/ocr-evaluation-results/provenance/source/evaluation/protocol.json"
write_evidence_manifest "$TEST_ROOT/cpu2/ocr-evaluation-results"
aggregate_fixture altered-protocol "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e '.provenance.shards[1].status=="rejected"' "$TEST_ROOT/altered-protocol/study-index.json" >/dev/null
cp -- "$REPOSITORY_ROOT/evaluation/protocol.json" "$TEST_ROOT/cpu2/ocr-evaluation-results/provenance/source/evaluation/protocol.json"
update_index "$TEST_ROOT/cpu2/ocr-evaluation-results" '.provenance.evaluator="../escape"'
write_evidence_manifest "$TEST_ROOT/cpu2/ocr-evaluation-results"
aggregate_fixture escaping-path "$TEST_ROOT/cpu1" "$TEST_ROOT/cpu2"
jq -e '.provenance.shards[1].status=="rejected"' "$TEST_ROOT/escaping-path/study-index.json" >/dev/null

# Selected-CPU topology is read literally from sysfs: on hosted runners the two
# selected CPUs are usually the two hardware threads of one core.
mkdir -p "$TEST_ROOT/sysfs/cpu0/topology" "$TEST_ROOT/sysfs/cpu1/topology" "$TEST_ROOT/sysfs/cpu2/topology" "$TEST_ROOT/sysfs/cpu3"
printf '0\n' > "$TEST_ROOT/sysfs/cpu0/topology/core_id"; printf '0-1\n' > "$TEST_ROOT/sysfs/cpu0/topology/thread_siblings_list"
printf '0\n' > "$TEST_ROOT/sysfs/cpu1/topology/core_id"; printf '0,1\n' > "$TEST_ROOT/sysfs/cpu1/topology/thread_siblings_list"
printf '1\n' > "$TEST_ROOT/sysfs/cpu2/topology/core_id"; printf 'two\n' > "$TEST_ROOT/sysfs/cpu2/topology/thread_siblings_list"
export MATRIX_SYSFS_CPU_ROOT=$TEST_ROOT/sysfs
jq -e '.cpu==0 and .core_id==0 and .thread_siblings_list=="0-1"' <<< "$(cpu_topology_json 0)" >/dev/null
jq -e '.cpu==1 and .core_id==0 and .thread_siblings_list=="0,1"' <<< "$(cpu_topology_json 1)" >/dev/null
jq -e '.cpu==3 and .core_id==null and .thread_siblings_list==null' <<< "$(cpu_topology_json 3)" >/dev/null
expect_failure cpu_topology_json 2
expect_failure cpu_topology_json one
unset MATRIX_SYSFS_CPU_ROOT

# Hermetic laboratory acquisition: a synthetic evaluator stub verifies every
# locked asset that is already present and answers HTTP 403, as the hosted
# runner observed on 2026-10-20, for any file a publisher would have to serve.
# The vendored PDFs and recovered HTML must make that network path moot.
cat > "$TEST_ROOT/mock-bin/indra-evaluate" <<'MOCK_EVALUATOR'
#!/usr/bin/env bash
# SYNTHETIC evaluator stub: never downloads, never runs OCR or scoring.
set -Eeuo pipefail
printf '%s\n' "$*" >> "$STUB_CALLS"
case ${1:-} in
  fetch)
    lock=''; root=''
    while (($#)); do
      case $1 in --lock) lock=$2; shift;; --root) root=$2; shift;; esac
      shift
    done
    while IFS=$'\t' read -r path digest bytes url; do
      if [[ -f $root/$path ]]; then
        [[ $(sha256sum < "$root/$path" | cut -d ' ' -f 1) == "$digest" && $(stat -c %s -- "$root/$path") == "$bytes" ]] ||
          { printf 'changed %s\n' "$path" >&2; exit 1; }
        printf 'verified %s\n' "$path" >&2
      else
        case $url in
          https://www.scielo.br/*|https://quimicanova.sbq.org.br/*)
            printf 'fetching %s\nError:   × source HTTP request failed\n  ╰─▶ HTTP status client error (403 Forbidden) for url (%s)\n' "$path" "$url" >&2
            exit 1;;
          *) printf 'stub-skipped %s\n' "$path" >&2;;
        esac
      fi
    done < <(jq -r '.assets[]|[.path,.sha256,.bytes,.url]|@tsv' "$lock");;
  prepare) printf '{"synthetic":true,"not_an_ocr_result":true,"verified_files":82,"failures":[]}\n';;
  verify) printf '{"synthetic":true,"not_an_ocr_result":true}\n';;
  *) exit 99;;
esac
MOCK_EVALUATOR
chmod u+x "$TEST_ROOT/mock-bin/indra-evaluate"
mkdir -p "$TEST_ROOT/stub-403/empty"
expect_failure env STUB_CALLS="$TEST_ROOT/stub-403/calls.txt" "$TEST_ROOT/mock-bin/indra-evaluate" fetch \
  --lock "$LAB/acquisition/execution-sources.json" --root "$TEST_ROOT/stub-403/empty" 2> "$TEST_ROOT/stub-403/stderr"
grep -Fq '403 Forbidden' "$TEST_ROOT/stub-403/stderr"
[[ -z $(find "$TEST_ROOT/stub-403/empty" -type f -print -quit) ]]

stage_acquisition_source() {
  local destination=$1 path
  mkdir -p "$destination/evaluation/fixtures"
  cp -R -- "$REPOSITORY_ROOT/evaluation/laboratory-supplement" "$REPOSITORY_ROOT/evaluation/acquisition" "$destination/evaluation/"
  cp -- "$REPOSITORY_ROOT/evaluation/corpus-frozen.json" "$REPOSITORY_ROOT/evaluation/protocol.json" "$destination/evaluation/"
  cp -- "$REPOSITORY_ROOT/evaluation/fixtures/legal-raster.json" "$destination/evaluation/fixtures/"
  while IFS= read -r path; do
    mkdir -p -- "$destination/${path%/*}"
    cp -- "$REPOSITORY_ROOT/$path" "$destination/$path"
  done < <(jq -r '.files[]|.repository_path' "$REPOSITORY_ROOT/evaluation/fixtures/legal-raster.json")
}
acquire_fixture() (
  local name=$1 output=$TEST_ROOT/$1 work=$TEST_ROOT/$1-work code=0
  mkdir -p "$output/control" "$output/provenance"
  stage_acquisition_source "$output/provenance/source"
  [[ ${2:-} != tampered ]] || printf '\n' >> "$output/provenance/source/evaluation/laboratory-supplement/fixtures/sources/hops.pdf"
  # A child shell keeps errexit semantics that a conditional call would disable.
  PATH="$TEST_ROOT/mock-bin:$PATH" STUB_CALLS="$output/stub-calls.txt" bash -Eeuo pipefail -c \
    'source "$1"; acquire_corpus_inputs "$2" "$3" "$4" true' _ \
    "$SCRIPT_DIRECTORY/evaluate-ocr-matrix.sh" "$output" "$work" "$TEST_ROOT/mock-bin/indra-evaluate" || code=$?
  return "$code"
)
acquire_fixture hermetic
[[ $(wc -l < "$TEST_ROOT/hermetic/control/laboratory-seeded-sources.tsv") == 4 ]]
jq -e '.exit_code==0' "$TEST_ROOT/hermetic/control/laboratory-source-fetch.status.json" >/dev/null
for seeded in sources/hops.pdf sources/chlorins.pdf sources/chlorins-pdf-route.html sources/chlorins-scielo.html; do
  grep -Fxq "verified $seeded" "$TEST_ROOT/hermetic/control/laboratory-source-fetch.stderr"
done
expect_failure grep -Eq 'fetching|403|stub-skipped' "$TEST_ROOT/hermetic/control/laboratory-source-fetch.stderr"
while IFS=$'\t' read -r path digest bytes; do
  verify_file "$TEST_ROOT/hermetic-work/laboratory/$path" "$digest" "$bytes"
done < <(jq -r '.assets[]|select(.path|endswith(".pdf"))|[.path,.sha256,.bytes]|@tsv' "$LAB/acquisition/execution-sources.json")
jq -e '.exit_code==0' "$TEST_ROOT/hermetic/control/laboratory-corpus-admission.status.json" >/dev/null
[[ $(grep -c '^fetch ' "$TEST_ROOT/hermetic/stub-calls.txt") == 4 ]]
# A vendored byte that differs from the operational lock stops acquisition
# before the evaluator fetch could substitute a publisher body.
expect_failure acquire_fixture tampered tampered
[[ ! -e $TEST_ROOT/tampered/control/laboratory-source-fetch.argv.json ]]
[[ ! -e $TEST_ROOT/tampered-work/laboratory/sources/hops.pdf ]]

TEST_COMPLETE=true
printf 'Offline matrix fixtures passed: fixed schedule, exact seed admission, finite command exits, owned model cleanup, nested manifests, recovered metadata, hermetic laboratory acquisition under a 403 publisher stub, selected-CPU topology, partial/completed path union and rejection paths. No OCR, network or Docker daemon executed.\n'
