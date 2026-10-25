#!/usr/bin/env bash
# Fixture-only ownership admission, not profiling or Docker execution.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
filter=$root/scripts/lib/profile-volume.jq
owned='[{"Name":"private-bundle","Driver":"local","Scope":"local","Options":null,"Labels":{"owner":"token"}}]'
check() { jq -e --arg name private-bundle --arg owner token --arg label owner -f "$filter" >/dev/null; }
printf '%s\n' "$owned" | check
for mutation in '.[0].Name="other"' '.[0].Driver="nfs"' '.[0].Scope="global"' \
    '.[0].Options={"device":"/host"}' '.[0].Options={}' '.[0].Labels.owner="other"' \
    '.[0].Labels={}' '.[0].Labels.extra="unadmitted"' '.=[]' '.+=[.[0]]'; do
    if printf '%s\n' "$owned" | jq "$mutation" | check; then
        printf 'Invalid volume admitted: %s\n' "$mutation" >&2
        exit 1
    fi
done
bash -n "$root/scripts/profile-native-docker.sh"
printf 'private volume fixture admission: valid owner accepted;10 invalid driver/options/owner/count cases rejected\n'
