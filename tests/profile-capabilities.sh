#!/usr/bin/env bash
# The same predicate is used by real container admission and these negative controls.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
admit() {
    jq -L "$root/scripts/lib" -e --arg phase "$1" \
        'include "profile-capabilities"; profile_capabilities($phase)' >/dev/null
}
for phase in tiff math; do
    for capability in PERFMON CAP_PERFMON; do
        printf '{"HostConfig":{"CapAdd":["%s"]}}\n' "$capability" | admit "$phase"
    done
    for caps in '[]' 'null' '["SYS_ADMIN"]' '["CAP_SYS_ADMIN"]' \
        '["CAP_PERFMON","SYS_ADMIN"]' '["PERFMON","CAP_PERFMON"]' '["perfmon"]' 'false'; do
        if printf '{"HostConfig":{"CapAdd":%s}}\n' "$caps" | admit "$phase" 2>/dev/null; then
            printf 'Unexpected profiling capabilities admitted: %s\n' "$caps" >&2
            exit 1
        fi
    done
done
for phase in gates criterion ablation; do
    printf '{"HostConfig":{"CapAdd":null}}\n' | admit "$phase"
    printf '{"HostConfig":{"CapAdd":[]}}\n' | admit "$phase"
    if printf '{"HostConfig":{"CapAdd":["CAP_PERFMON"]}}\n' | admit "$phase"; then
        printf 'Unneeded sampling capability admitted for %s\n' "$phase" >&2
        exit 1
    fi
done
bash -n "$root/scripts/profile-native-docker.sh"
printf 'profiling capability aliases accepted; extra, duplicate, wrong and unneeded capabilities rejected\n'
