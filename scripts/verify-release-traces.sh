#!/usr/bin/env bash
# Read-only admission of the real trace gate against the shipped binary bytes.
set -euo pipefail
[[ $# = 1 ]] || exit 2
evidence=$1
regular() { [[ -f $1 && ! -L $1 && $(stat -c %s -- "$1") -le $2 ]]; }
for cpus in 1 2; do
    run="$evidence/tracing/$cpus-cpu/run"
    regular "$run/trace-acceptance.json" 65536
    regular "$run/assets.jsonl" 65536
    jq -se 'length==1 and (.[0]|.passed==true and .graph_verified==true and
        .orphan_reaping_complete==true and .gateway_exit==0 and .native_exit==0)' \
        "$run/trace-acceptance.json" > /dev/null
    for pair in 'gateway bin/indra-gateway' 'native bin/indra-server' \
        'bridge bin/indra-tesseract' 'patched_tika materials/tika/java/tika-server.jar'; do
        read -r role relative <<< "$pair"
        regular "$evidence/$relative" 536870912
        hash=$(sha256sum -- "$evidence/$relative" | cut -d' ' -f1)
        jq -se --arg role "$role" --arg hash "$hash" '
            map(select(.role==$role)) | length==1 and .[0].sha256==$hash
        ' "$run/assets.jsonl" > /dev/null
    done
done
printf '{"passed":true,"cpu_caps":[1,2],"scope":"complete trace graph and exact shipped binary/JAR identities"}\n'
