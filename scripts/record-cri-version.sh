#!/usr/bin/env bash
# Record both APIs without pretending crictl's plaintext is tool-emitted JSON.
# kind v0.33.0/images/base/Dockerfile pins cri-tools v1.36.0. Its
# cmd/crictl/version.go has no output flag; `version` queries the runtime,
# whereas `--version` reports the client. Its pinned Makefile strips the tag
# prefix before linking pkg/version.Version, so the CLI prints 1.36.0, not v1.36.0.
# Keep their raw streams separately.
set -euo pipefail
[[ $# -ge 2 ]] || { echo 'Usage: record-cri-version.sh NEW_OUTPUT COMMAND [ARG ...]' >&2; exit 2; }
output=$1
shift
mkdir -- "$output"
timeout --signal=TERM --kill-after=5s 30s "$@" version \
    > "$output/runtime.stdout.txt" 2> "$output/runtime.stderr.txt"
timeout --signal=TERM --kill-after=5s 30s "$@" --version \
    > "$output/client.stdout.txt" 2> "$output/client.stderr.txt"
jq -en --rawfile runtime "$output/runtime.stdout.txt" --rawfile client "$output/client.stdout.txt" '
  def lines:
    split("\n") | if .[-1] == "" then .[:-1] else . end
    | map(sub("\r$"; ""));
  def field($label): capture("^" + $label + ":[ \\t]+(?<value>[^ \\t\\r\\n]+)[ \\t]*$").value;
  ($runtime | lines | select(length == 4)) as $r
  | ($client | lines | select(length == 1) | .[0]
     | capture("^crictl version (?<version>[0-9]+[.][0-9]+[.][0-9]+)$").version) as $client_version
  | select($client_version == "1.36.0")
  | {schema_version:1, format:"parsed-crictl-plaintext", client_version:$client_version,
     runtime:{version:($r[0] | field("Version")), name:($r[1] | field("RuntimeName")),
       version_string:($r[2] | field("RuntimeVersion")), api_version:($r[3] | field("RuntimeApiVersion"))},
     raw:{runtime_stdout:"runtime.stdout.txt",runtime_stderr:"runtime.stderr.txt",
       client_stdout:"client.stdout.txt",client_stderr:"client.stderr.txt"}}
' > "$output/result.json"
