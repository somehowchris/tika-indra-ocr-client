#!/usr/bin/env bash
# Synthetic admission fixtures, not actual Tika/OTLP execution.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
owned=$(mktemp -d)
trap 'rm -rf -- "$owned"' EXIT
base() {
    rm -rf -- "$owned/evidence"
    mkdir -p "$owned/evidence/bin" "$owned/evidence/materials/tika/java"
    for pair in 'gateway bin/indra-gateway' 'native bin/indra-server' \
        'bridge bin/indra-tesseract' 'patched_tika materials/tika/java/tika-server.jar'; do
        read -r role file <<< "$pair"
        printf '%s fixture\n' "$role" > "$owned/evidence/$file"
    done
    for cpu in 1 2; do
        run="$owned/evidence/tracing/$cpu-cpu/run"
        mkdir -p "$run"
        printf '{"passed":true,"graph_verified":true,"orphan_reaping_complete":true,"gateway_exit":0,"native_exit":0}\n' > "$run/trace-acceptance.json"
        for pair in 'gateway bin/indra-gateway' 'native bin/indra-server' \
            'bridge bin/indra-tesseract' 'patched_tika materials/tika/java/tika-server.jar'; do
            read -r role file <<< "$pair"
            jq -nc --arg role "$role" --arg hash "$(sha256sum "$owned/evidence/$file" | cut -d' ' -f1)" \
                '{role:$role,sha256:$hash}' >> "$run/assets.jsonl"
        done
    done
}
checks=0
check() {
    status=0
    bash "$root/scripts/verify-release-traces.sh" "$owned/evidence" > "$owned/stdout" 2> "$owned/stderr" || status=$?
    if [[ $1 = pass ]]; then [[ $status = 0 ]]; else [[ $status != 0 ]]; fi
    checks=$((checks+1))
}
base; check pass
base; printf 'wrong binary\n' >> "$owned/evidence/bin/indra-server"; check fail
base; rm "$owned/evidence/tracing/2-cpu/run/trace-acceptance.json"; check fail
base; sed -i 's/"graph_verified":true/"graph_verified":false/' "$owned/evidence/tracing/1-cpu/run/trace-acceptance.json"; check fail
base; sed -i 's/"passed":true/"passed":"true"/' "$owned/evidence/tracing/1-cpu/run/trace-acceptance.json"; check fail
base; cat "$owned/evidence/tracing/1-cpu/run/trace-acceptance.json" >> "$owned/evidence/tracing/2-cpu/run/trace-acceptance.json"; check fail
base; head -1 "$owned/evidence/tracing/1-cpu/run/assets.jsonl" >> "$owned/evidence/tracing/2-cpu/run/assets.jsonl"; check fail
base; sed -i 's/"native_exit":0/"native_exit":9/' "$owned/evidence/tracing/2-cpu/run/trace-acceptance.json"; check fail
base; mv "$owned/evidence/bin/indra-server" "$owned/server"; ln -s "$owned/server" "$owned/evidence/bin/indra-server"; check fail
base; printf 'invalid JSON\n' >> "$owned/evidence/tracing/1-cpu/run/assets.jsonl"; check fail
printf '{"scope":"synthetic release trace admission fixtures","passed":%s}\n' "$checks"
