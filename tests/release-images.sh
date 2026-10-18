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
