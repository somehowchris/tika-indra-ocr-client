#!/usr/bin/env bash
# Controller fixtures use fake Docker and daemon-binding transports. The raw
# OCI-chain validator has its own tests. These prove mandatory delegation,
# root/platform separation, cleanup and terminal-state rejection, not execution.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$temporary/bin" "$temporary/controller/scripts"
cp "$root/scripts/verify-release-images.sh" "$temporary/controller/scripts/verify-release-images.sh"
cp "$root/scripts/verify-release-traces.sh" "$temporary/controller/scripts/verify-release-traces.sh"
export GITHUB_SHA=0123456789012345678901234567890123456789
cat > "$temporary/bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
command=$1
shift
case "$command" in
  create)
    image= gate=models
    for value in "$@"; do
      if [[ "$value" =~ ^sha256:[a-f0-9]{64}$ ]]; then image=$value; fi
      if [[ "$value" = /gates/* ]]; then gate=${value#/gates/}; fi
      [[ "$value" != *-acceptance:test ]] || exit 90
    done
    [[ -n "$image" ]]
    count=0
    [[ ! -e "$RELEASE_TEST_STATE/count" ]] || read -r count < "$RELEASE_TEST_STATE/count"
    count=$((count + 1))
    printf '%s\n' "$count" > "$RELEASE_TEST_STATE/count"
    printf -v id '%064x' "$count"
    jq -n --arg id "$id" --arg image "$image" --arg gate "$gate" \
      --slurpfile source "$RELEASE_TEST_STATE/images/${image#sha256:}.json" \
      '{Id:$id,Image:$image,gate:$gate,ImageManifestDescriptor:{digest:$source[0].platform_digest,
        size:$source[0].platform_size,mediaType:$source[0].platform_media_type,platform:{os:"linux",architecture:"amd64"}}}' \
      > "$RELEASE_TEST_STATE/$id.json"
    printf '%s\t%s\t%s\n' "$id" "$image" "$gate" >> "$RELEASE_TEST_STATE/created.tsv"
    printf '%s\n' "$id";;
  start)
    [[ $# = 2 && $1 = --attach && $2 =~ ^[a-f0-9]{64}$ ]]
    gate=$(jq -r .gate "$RELEASE_TEST_STATE/$2.json")
    printf '%s\n' "$2" >> "$RELEASE_TEST_STATE/started.txt"
    if [[ ( "$RELEASE_TEST_MODE" = model-timeout && "$gate" = models ) ||
          ( "$RELEASE_TEST_MODE" = command-timeout && "$gate" = server-unit ) ]]; then exit 124; fi
    printf '{"test_double":true}\n';;
  inspect)
    [[ $# = 1 && $1 =~ ^[a-f0-9]{64}$ ]]
    jq --arg mode "$RELEASE_TEST_MODE" '
      {Id,Image,ImageManifestDescriptor,RestartCount:0,State:{Running:false,OOMKilled:false,ExitCode:0}} as $state |
      [if .gate == "server-unit" then
        $state | if $mode == "wrong-image" then .Image = ("sha256:"+("f"*64))
        elif $mode == "wrong-platform" then .ImageManifestDescriptor.digest = ("sha256:"+("f"*64))
        elif $mode == "missing-descriptor" then del(.ImageManifestDescriptor)
        elif $mode == "oom" then .State.OOMKilled = true
        elif $mode == "nonzero" then .State.ExitCode = 9
        elif $mode == "restarted" then .RestartCount = 1
        elif $mode == "running" then .State.Running = true else . end
      else $state end]
    ' "$RELEASE_TEST_STATE/$1.json";;
  rm)
    [[ $1 != --force ]] || shift
    [[ $# = 1 && $1 =~ ^[a-f0-9]{64}$ && -f "$RELEASE_TEST_STATE/$1.json" ]]
    printf '%s\n' "$1" >> "$RELEASE_TEST_STATE/removed.txt";;
  *) exit 91;;
esac
SH
cat > "$temporary/controller/scripts/verify-daemon-image.sh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ $# = 4 && $1 = bind && ! -e $4 ]]
printf '%s\t%s\n' "$2" "$3" >> "$RELEASE_TEST_STATE/bindings.tsv"
if [[ "$RELEASE_TEST_MODE" = binding-failure && "$3" = *server-unit* ]]; then exit 18; fi
jq -e --slurpfile identity "$2/result.json" '
  length == 1 and .[0].Image == $identity[0].root_digest and
  .[0].ImageManifestDescriptor.digest == $identity[0].platform_digest and
  .[0].ImageManifestDescriptor.size == $identity[0].platform_size and
  .[0].ImageManifestDescriptor.mediaType == $identity[0].platform_media_type and
  .[0].ImageManifestDescriptor.platform == {os:"linux",architecture:"amd64"}
' "$3" > /dev/null
mkdir "$4"
printf '{"passed":true,"test_double":true}\n' > "$4/result.json"
SH
chmod +x "$temporary/bin/docker"
export PATH="$temporary/bin:$PATH"

resolution() {
  local index=$1 output=$2 digest platform config
  mkdir -p "$output"
  printf -v digest '%064x' "$index"
  printf -v platform '%064x' "$((index + 100))"
  printf -v config '%064x' "$((index + 200))"
  jq -n --arg root "sha256:$digest" --arg platform "sha256:$platform" --arg config "sha256:$config" '
    {passed:true,store:"containerd",image_id:$root,root_digest:$root,platform_digest:$platform,
     platform_size:333,platform_media_type:"application/vnd.oci.image.manifest.v1+json",
     config_digest:$config,os:"linux",architecture:"amd64"}
  ' > "$output/result.json"
  cp "$output/result.json" "$RELEASE_TEST_STATE/images/$digest.json"
}

run_case() {
  local name=$1 expected=$2 status=0 evidence="$temporary/$1/evidence" index=0 digest
  export RELEASE_TEST_MODE="$name" RELEASE_TEST_STATE="$temporary/$name/state"
  mkdir -p "$evidence/build" "$evidence/images" "$evidence/acceptance-build" "$RELEASE_TEST_STATE/images"
  printf '{"passed":true}\n' > "$evidence/prepare-result.json"
  for role in native gateway bridge tika tesseract backend; do
    index=$((index + 1))
    printf -v digest '%064x' "$index"
    mkdir "$evidence/build/$role"
    jq -n --arg role "$role" --arg digest "$digest" \
      '{role:$role,reference:("ghcr.io/example/tika-indra-"+$role+"@sha256:"+$digest)}' > "$evidence/build/$role/image.json"
    resolution "$index" "$evidence/images/$role-resolution"
  done
  for role in native gateway; do
    index=$((index + 1))
    printf -v digest '%064x' "$index"
    resolution "$index" "$evidence/acceptance-build/$role/resolution"
  done
  if [[ "$name" = invalid-acceptance ]]; then
    printf '{"passed":true,"store":"containerd","root_digest":"indra-server-acceptance:test","image_id":"indra-server-acceptance:test","architecture":"amd64","os":"linux"}\n' \
      > "$evidence/acceptance-build/native/resolution/result.json"
  fi
  bash "$temporary/controller/scripts/verify-release-images.sh" process "$evidence" \
    > "$temporary/$name/stdout" 2> "$temporary/$name/stderr" || status=$?
  if [[ "$expected" = pass ]]; then
    [[ "$status" = 0 && $(wc -l < "$RELEASE_TEST_STATE/created.tsv") = 10 && $(wc -l < "$RELEASE_TEST_STATE/bindings.tsv") = 10 ]]
    jq -e '.passed and .cpu_caps == [1,2]' "$evidence/process/result.json" > /dev/null
    # Root IDs differ from platform and config IDs in every fixture. They
    # identify the Docker invocation; mandatory bind checks the selected child.
    awk -F '\t' '
      $3 == "models" && $2 !~ /0001$/ {exit 1}
      $3 ~ /^server-/ && $2 !~ /0007$/ {exit 1}
      $3 ~ /^gateway-/ && $2 !~ /0008$/ {exit 1}
    ' "$RELEASE_TEST_STATE/created.tsv"
  else
    [[ "$status" != 0 && ! -e "$evidence/process/result.json" ]]
  fi
  if [[ "$name" = invalid-acceptance ]]; then
    [[ ! -e "$RELEASE_TEST_STATE/created.tsv" ]]
  else
    cut -f1 "$RELEASE_TEST_STATE/created.tsv" | sort > "$temporary/$name/created.txt"
    sort "$RELEASE_TEST_STATE/removed.txt" > "$temporary/$name/removed.txt"
    cmp "$temporary/$name/created.txt" "$temporary/$name/removed.txt"
  fi
}

run_case pass pass
for name in wrong-image wrong-platform missing-descriptor binding-failure oom nonzero restarted running command-timeout model-timeout invalid-acceptance; do
  run_case "$name" fail
done
printf 'Release process controller: 12 cases passed; mandatory binding, distinct root/platform/config identities, terminal failures, command deadlines and owned cleanup. No real Docker/native execution claim.\n'

# Exercise index mode with the real trace-binding validator. These tiny fixture
# assets/admissions prove controller checks, not actual tracing or OCR execution.
index_command() {
  env -u root GITHUB_REPOSITORY=example/tika-indra-ocr-client GITHUB_REF=refs/tags/v-test \
    GITHUB_RUN_ID=42 GITHUB_RUN_ATTEMPT=1 \
    GITHUB_WORKFLOW_REF=example/tika-indra-ocr-client/.github/workflows/release.yml@refs/tags/v-test \
    bash "$temporary/controller/scripts/verify-release-images.sh" index "$1"
}

run_index_case() {
  local name=$1 expected=$2 status=0 evidence="$temporary/$1/evidence" run role relative cpus pair manifest_sha
  mkdir -p "$temporary/$name"
  cp -a "$temporary/pass/evidence" "$evidence"
  mkdir -p "$evidence/references" "$evidence/bin" "$evidence/materials/tika/java" "$evidence/models"
  printf '{"passed":true}\n' > "$evidence/references/result.json"
  for role in compose kubernetes; do
    mkdir "$evidence/$role"
    printf '0\n' > "$evidence/$role/exit-status.txt"
    printf 'fixture gate passed\n' > "$evidence/$role/result.txt"
  done
  printf 'nested material manifest\n' > "$evidence/materials/SHA256SUMS"
  printf 'excluded reusable cache\n' > "$evidence/models/cache.bin"
  for cpus in 1 2; do
    run="$evidence/tracing/$cpus-cpu/run"
    mkdir -p "$run"
    printf '{"passed":true,"graph_verified":true,"orphan_reaping_complete":true,"gateway_exit":0,"native_exit":0}\n' > "$run/trace-acceptance.json"
    for pair in 'gateway bin/indra-gateway' 'native bin/indra-server' \
      'bridge bin/indra-tesseract' 'patched_tika materials/tika/java/tika-server.jar'; do
      read -r role relative <<< "$pair"
      printf '%s\n' "$role fixture bytes" > "$evidence/$relative"
      jq -nc --arg role "$role" --arg hash "$(sha256sum "$evidence/$relative" | cut -d' ' -f1)" \
        '{role:$role,sha256:$hash}' >> "$run/assets.jsonl"
    done
  done
  if [[ "$name" = index-bad-trace ]]; then
    jq '.graph_verified=false' "$run/trace-acceptance.json" > "$temporary/$name/invalid.json"
    mv "$temporary/$name/invalid.json" "$run/trace-acceptance.json"
  elif [[ "$name" = index-bad-asset ]]; then
    jq -c 'if .role=="native" then .sha256=("f"*64) else . end' "$run/assets.jsonl" > "$temporary/$name/invalid.json"
    mv "$temporary/$name/invalid.json" "$run/assets.jsonl"
  fi
  index_command "$evidence" > "$temporary/$name/stdout" 2> "$temporary/$name/stderr" || status=$?
  if [[ "$expected" = pass ]]; then
    [[ "$status" = 0 ]]
    jq -e '.passed and .cpu_caps==[1,2]' "$evidence/tracing/release-binding.json" > /dev/null
    jq -e --arg sha "$GITHUB_SHA" '.source_sha==$sha and .release_run_id=="42" and .release_run_attempt=="1" and (.images|length)==6' "$evidence/candidates.json" > /dev/null
    manifest_sha=$(sha256sum "$evidence/evidence-files.sha256" | cut -d' ' -f1)
    jq -e --arg sha "$manifest_sha" '.verification.passed and .evidence_manifest.sha256==$sha and .scope.performance_claim==false' "$evidence/release-predicate.json" > /dev/null
    (cd "$evidence"; sha256sum --check evidence-files.sha256) > "$temporary/$name/manifest-check.txt"
    grep -Fq '  ./materials/SHA256SUMS' "$evidence/evidence-files.sha256"
    ! grep -Fq './models/' "$evidence/evidence-files.sha256"
    cp "$evidence/evidence-files.sha256" "$temporary/$name/index-before.sha256"
    status=0
    index_command "$evidence" > "$temporary/$name/repeat.stdout" 2> "$temporary/$name/repeat.stderr" || status=$?
    [[ "$status" = 1 ]]
    grep -Fxq 'refusing to overwrite the release index' "$temporary/$name/repeat.stderr"
    cmp "$temporary/$name/index-before.sha256" "$evidence/evidence-files.sha256"
  else
    [[ "$status" = 1 && ! -e "$evidence/candidates.json" && ! -e "$evidence/evidence-files.sha256" && ! -e "$evidence/release-predicate.json" ]]
  fi
}

run_index_case index-pass pass
run_index_case index-bad-trace fail
run_index_case index-bad-asset fail
printf 'Release index controller: complete index and manifest verification passed; invalid second-cap trace, changed asset and index overwrite rejected with root unset. Fixture evidence only.\n'
