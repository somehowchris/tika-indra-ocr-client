#!/usr/bin/env bash
# Exercise the recorder against the pinned crictl CLI contract, without Docker.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
cat > "$work/crictl" <<'CLI'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$CALL_LOG"
case "$*" in
    version)
        case "${MODE:-valid}" in
            valid) cat "$CAPTURED_CRI/runtime.stdout.txt"; exit ;;
            failure) echo 'runtime unavailable' >&2; exit 1 ;;
            missing) printf 'Version:  0.1.0\nRuntimeName:  containerd\nRuntimeVersion:  2.2.0\n'; exit ;;
            duplicate) printf 'Version:  0.1.0\nRuntimeName:  containerd\nRuntimeVersion:  2.2.0\nRuntimeVersion:  v1\n'; exit ;;
            extra) printf 'unexpected line\n' ;;
        esac
        printf 'Version:  0.1.0\nRuntimeName:  containerd\nRuntimeVersion:  2.2.0\nRuntimeApiVersion:  v1\n'
        ;;
    --version)
        if [[ -n ${CLIENT_VERSION+x} ]]; then printf 'crictl version %s\n' "$CLIENT_VERSION"
        else cat "$CAPTURED_CRI/client.stdout.txt"; fi ;;
    *) echo 'flag provided but not defined: -o' >&2; exit 1 ;;
esac
CLI
chmod +x "$work/crictl"
export CALL_LOG="$work/calls.txt"
export CAPTURED_CRI="$root/tests/fixtures/cri-version-1.36.0"
# The original invocation must fail: a fake accepting every argument hides this regression.
if "$work/crictl" version -o json > "$work/old.stdout" 2> "$work/old.stderr"; then exit 1; fi
grep -Fxq 'flag provided but not defined: -o' "$work/old.stderr"
: > "$CALL_LOG"
bash "$root/scripts/record-cri-version.sh" "$work/valid" "$work/crictl"
printf 'version\n--version\n' > "$work/expected-calls.txt"
cmp "$CALL_LOG" "$work/expected-calls.txt"
jq -e '.format == "parsed-crictl-plaintext" and .client_version == "1.36.0" and
  .runtime == {version:"0.1.0",name:"containerd",version_string:"v2.3.4",api_version:"v1"}' "$work/valid/result.json" > /dev/null
for mode in missing duplicate extra failure; do
    if MODE="$mode" bash "$root/scripts/record-cri-version.sh" "$work/$mode" "$work/crictl"; then
        echo "incorrectly accepted $mode runtime reply" >&2; exit 1
    fi
done
grep -Fxq 'runtime unavailable' "$work/failure/runtime.stderr.txt"
for version in 1.35.0 v1.36.0 1.36.0-extra; do
    if CLIENT_VERSION="$version" bash "$root/scripts/record-cri-version.sh" "$work/wrong-client-$version" "$work/crictl"; then exit 1; fi
done
if bash "$root/scripts/record-cri-version.sh" "$work/valid" "$work/crictl"; then exit 1; fi
printf '%s\n' 'CRI version contract fixtures passed: separate client/runtime, unsupported flag, malformed reply, command failure, wrong client and overwrite.'
