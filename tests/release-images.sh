#!/usr/bin/env bash
# Controller fixtures use a fake Docker transport. They prove exact-ID
# admission/cleanup and terminal-state rejection, not real native execution.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/bin"
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
      '{Id:$id,Image:$image,gate:$gate}' > "$RELEASE_TEST_STATE/$id.json"
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
      {Id,Image,RestartCount:0,State:{Running:false,OOMKilled:false,ExitCode:0}} as $state |
      [if .gate == "server-unit" then
        $state | if $mode == "wrong-image" then .Image = ("sha256:"+("f"*64))
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
chmod +x "$temporary/bin/docker"
export PATH="$temporary/bin:$PATH"

run_case() {
  local name=$1 expected=$2 status=0 evidence="$temporary/$1/evidence" index=0 digest
  export RELEASE_TEST_MODE="$name" RELEASE_TEST_STATE="$temporary/$name/state"
  mkdir -p "$evidence/build" "$evidence/images" "$evidence/acceptance-build" "$RELEASE_TEST_STATE"
  printf '{"passed":true}\n' > "$evidence/prepare-result.json"
  for role in native gateway bridge tika tesseract backend; do
    index=$((index + 1))
    printf -v digest '%064x' "$index"
    mkdir "$evidence/build/$role"
    jq -n --arg role "$role" --arg digest "$digest" \
      '{role:$role,reference:("ghcr.io/example/tika-indra-"+$role+"@sha256:"+$digest)}' > "$evidence/build/$role/image.json"
    jq -n --arg digest "$digest" '[{Id:("sha256:"+$digest)}]' > "$evidence/images/$role.json"
  done
  for role in native gateway; do
    index=$((index + 1))
    printf -v digest '%064x' "$index"
    mkdir "$evidence/acceptance-build/$role"
    jq -n --arg digest "$digest" '[{Id:("sha256:"+$digest),Architecture:"amd64",Os:"linux"}]' \
      > "$evidence/acceptance-build/$role/image.json"
  done
  if [[ "$name" = invalid-acceptance ]]; then
    printf '[{"Id":"indra-server-acceptance:test","Architecture":"amd64","Os":"linux"}]\n' \
      > "$evidence/acceptance-build/native/image.json"
  fi
  bash "$root/scripts/verify-release-images.sh" process "$evidence" \
    > "$temporary/$name/stdout" 2> "$temporary/$name/stderr" || status=$?
  if [[ "$expected" = pass ]]; then
    [[ "$status" = 0 && $(wc -l < "$RELEASE_TEST_STATE/created.tsv") = 10 ]]
    jq -e '.passed and .cpu_caps == [1,2]' "$evidence/process/result.json" > /dev/null
    # Each role's stored config ID is used for both CPU budgets, irrespective
    # of any mutable acceptance tag. Native model fetch uses its saved ID too.
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
for name in wrong-image oom nonzero restarted running command-timeout model-timeout invalid-acceptance; do
  run_case "$name" fail
done
printf 'Release process controller: nine cases passed; recorded image IDs, terminal failure rejection, bounded-command status, and exact owned-container cleanup. No real Docker/native execution claim.\n'
