#!/usr/bin/env bash
# Offline controller plan tests: never inference, Docker or measurements.
set -Eeuo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
parent=$(mktemp -d "${TMPDIR:-/tmp}/indra-continuous-fixture.XXXXXXXX")
trap 'rm -rf -- "$parent"' EXIT
bash "$root/scripts/benchmark-continuous.sh" plan "$parent/one"
bash "$root/scripts/benchmark-continuous.sh" plan "$parent/two"
cmp "$parent/one/study-index.json" "$parent/two/study-index.json"
jq -e '(.cells|length)==240 and ([.cells[]|[.layer,.cpus,.concurrency,.repetition,.arm]]|unique|length)==240 and
  all(.cells[];.status=="planned" and .exit_code==null and .summary==null) and
  ([.cells[]|select(.layer=="native")]|length)==120 and ([.cells[]|select(.layer=="tika")]|length)==120 and
  all(.cells|group_by(.layer,.cpus,.concurrency,.repetition)[]; length==2 and
    (if (.[0].repetition%2)==0 then [.[].arm]==["ordinary","continuous"] else [.[].arm]==["continuous","ordinary"] end))' "$parent/one/study-index.json" >/dev/null
if bash "$root/scripts/benchmark-continuous.sh" plan "$parent/one" > "$parent/existing.stdout" 2> "$parent/existing.stderr"; then
  printf 'An existing experiment directory was overwritten\n' >&2; exit 1
fi
if bash "$root/scripts/benchmark-continuous.sh" unknown > "$parent/unknown.stdout" 2> "$parent/unknown.stderr"; then
  printf 'An invalid controller mode was accepted\n' >&2; exit 1
fi
printf 'Offline plan identities, all240 cells, paired arm order and rejection paths passed; no measurements.\n'
