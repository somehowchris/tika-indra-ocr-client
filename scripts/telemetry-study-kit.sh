#!/usr/bin/env bash
# Assemble the inputs of the telemetry-overhead study from one signed release
# run, so an operator runs scripts/benchmark-telemetry-http.sh on a host without
# building or hashing anything: the static driver, the trace-acceptance image it
# runs in, the identity the controller requires and a native-equivalence receipt
# bound to this run's native process gates and signing. Model weights are not
# redistributed: the operator fetches them with the image's own native server.
set -euo pipefail
export LC_ALL=C
umask 022
fail() { printf 'telemetry study kit: %s\n' "$*" >&2; exit 1; }
[[ $# = 4 ]] || fail 'usage: telemetry-study-kit.sh TRACE_IMAGE GATEWAY_ACCEPTANCE_IMAGE RELEASE_EVIDENCE NEW_OUTPUT'
image=$1
acceptance=$2
evidence=$(realpath -e -- "$3")
output=$(realpath -m -- "$4")
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
sha() { sha256sum -- "$1" | cut -d' ' -f1; }

[[ ! -e $output && ! -L $output ]] || fail 'output must not already exist'
[[ ${GITHUB_REPOSITORY:-} && ${GITHUB_RUN_ID:-} =~ ^[0-9]+$ && ${GITHUB_RUN_ATTEMPT:-} =~ ^[0-9]+$ ]] \
    || fail 'the GitHub run identity is required'
source_sha=$(git -C "$root" rev-parse HEAD)
[[ $source_sha = "${GITHUB_SHA:-}" ]] || fail 'the checkout is not the released source'
git -C "$root" diff --quiet HEAD -- runtime shared scripts || fail 'the released source tree is not clean'
jq -e '.passed == true and .cpu_caps == [1,2]' "$evidence/process/result.json" > /dev/null \
    || fail 'the native and gateway process gates of this run did not pass'
jq -e '.passed == true and .images == 6' "$evidence/signing/result.json" > /dev/null \
    || fail 'the six candidates of this run are not signed'
mkdir -p -- "$(dirname -- "$output")"
mkdir -m 0755 -- "$output"
mkdir "$output/roles" "$output/roles/trace"

# 1. The static driver from this run's verified gateway acceptance build, which
#    compiles it with the shipped gateway's toolchain and static checks.
acceptance_id=$(docker image inspect --format '{{.Id}}' "$acceptance")
[[ $acceptance_id = "$(jq -er '.root_digest' "$evidence/acceptance-build/gateway/resolution/result.json")" ]] \
    || fail 'the gateway acceptance image is not the one this run verified'
container=$(docker create --pull never "$acceptance_id")
trap 'docker rm --force "$container" > /dev/null 2>&1 || true' EXIT
docker cp "$container:/indra-telemetry-http" "$output/telemetry_http"
docker cp "$container:/build-evidence/binaries.sha256" "$output/driver-build.sha256"
docker rm "$container" > /dev/null
trap - EXIT
chmod 0755 "$output/telemetry_http"
grep -qx "$(sha "$output/telemetry_http")  /out/indra-telemetry-http" "$output/driver-build.sha256" \
    || fail 'the driver is not the one the gateway build recorded'
readelf -W -l "$output/telemetry_http" > "$output/driver.program-headers.txt"
readelf -W -d "$output/telemetry_http" > "$output/driver.dynamic.txt"
if grep -q INTERP "$output/driver.program-headers.txt" || grep -q NEEDED "$output/driver.dynamic.txt"; then
    fail 'the driver is not a static GNU ELF'
fi

# 2. The role files exactly as the driver hashes them inside the assembled image.
image_id=$(docker image inspect --format '{{.Id}}' "$image")
[[ $image_id =~ ^sha256:[0-9a-f]{64}$ ]] || fail 'the assembled image has no content-addressed ID'
container=$(docker create --pull never "$image_id")
trap 'docker rm --force "$container" > /dev/null 2>&1 || true' EXIT
declare -A paths=([gateway]=/indra-gateway [native]=/indra-server [java]=/usr/bin/java
    [tika]=/opt/tika/tika-server.jar [bridge]=/opt/indra/tesseract)
for role in gateway native java tika bridge; do
    docker cp --follow-link "$container:${paths[$role]}" "$output/roles/$role"
done
docker cp "$container:/opt/tika/trace/." "$output/roles/trace/"
docker rm "$container" > /dev/null
trap - EXIT
[[ $(find "$output/roles/trace" -mindepth 1 -maxdepth 1 -type f | wc -l) = 16 &&
   -z $(find "$output/roles/trace" -mindepth 1 ! -type f -print -quit) ]] \
    || fail 'the trace extension closure is not exactly 16 regular files'
# The assembled roles are the release-verified, signed binaries.
cmp "$output/roles/native" "$evidence/bin/indra-server" || fail 'native role differs from the signed native server'
cmp "$output/roles/gateway" "$evidence/bin/indra-gateway" || fail 'gateway role differs from the signed gateway'
cmp "$output/roles/bridge" "$evidence/bin/indra-tesseract" || fail 'bridge role differs from the signed bridge'

# 3. The saved image must load under the same content-addressed ID.
# The whole image, not one platform: its root descriptor is the ID docker load restores.
docker save --output "$output/trace-acceptance.tar" "$image"
saved=$(tar -xOf "$output/trace-acceptance.tar" index.json | jq -er '.manifests | select(length == 1) | .[0].digest')
[[ $saved = "$image_id" ]] || fail "saved image index $saved does not match $image_id"

# 4. The receipt binds the native role to this run's real gates and signatures.
gates=$(cd "$evidence" && find process -maxdepth 1 -name '*-[12].log' -type f -print0 | sort -z \
    | xargs -0 sha256sum | jq -Rn '[inputs | capture("^(?<sha256>[0-9a-f]{64})  (?<path>.+)$")]')
[[ $(jq length <<< "$gates") = 8 ]] || fail 'expected four process gates at two CPU caps'
jq -n --arg repository "$GITHUB_REPOSITORY" --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" \
    --arg source "$source_sha" --arg native "$(sha "$output/roles/native")" \
    --arg reference "$(jq -er .reference "$evidence/build/native/image.json")" \
    --arg process "$(sha "$evidence/process/result.json")" --arg signing "$(sha "$evidence/signing/result.json")" \
    --arg materials "$(sha "$evidence/materials/native/SHA256SUMS")" --argjson gates "$gates" '{
    schema_version: 1,
    scope: "Signed release candidate: the native server in the assembled image is byte-identical to the signed native image whose real unit and process gates passed at 1 and 2 CPUs in this run. Not a performance result.",
    release: {repository: $repository, run_id: $run, run_attempt: $attempt, source_sha: $source},
    native_reference: $reference, native_server_sha256: $native, native_materials_sha256: $materials,
    process_result_sha256: $process, signing_result_sha256: $signing,
    gate_logs: $gates}' > "$output/native-equivalence-receipt.json"

roles=$(jq -n --arg gateway "$(sha "$output/roles/gateway")" --arg native "$(sha "$output/roles/native")" \
    --arg java "$(sha "$output/roles/java")" --arg tika "$(sha "$output/roles/tika")" \
    --arg bridge "$(sha "$output/roles/bridge")" '{gateway: $gateway, native: $native, java: $java, tika: $tika, bridge: $bridge}')
trace=$(cd "$output/roles/trace" && find . -maxdepth 1 -type f -print0 | sort -z | xargs -0 sha256sum \
    | jq -Rn '[inputs | capture("^(?<v>[0-9a-f]{64})  \\./(?<k>.+)$") | {(.k): .v}] | add')
jq -n --arg source "$source_sha" --arg binary "$(sha "$output/telemetry_http")" \
    --arg protocol "$(sha "$root/runtime/validation/telemetry-http-protocol.json")" --arg image "$image_id" \
    --argjson roles "$roles" --argjson trace "$trace" \
    --arg materials "$(sha "$evidence/materials/native/SHA256SUMS")" \
    --arg receipt "$(sha "$output/native-equivalence-receipt.json")" \
    --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" '{
    schema_version: 1, source_sha: $source, binary_sha256: $binary, protocol_sha256: $protocol,
    assembled_image_root: $image, role_sha256: $roles, trace_dependencies_sha256: $trace,
    native_materials_sha256: $materials, native_equivalence_passed: true,
    native_equivalence_receipt: "native-equivalence-receipt.json", native_equivalence_receipt_sha256: $receipt,
    acceptance_scope: "Signed release roles assembled in the release trace-acceptance image; driver built from the released source.",
    release_run_id: $run, release_run_attempt: $attempt}' > "$output/identity.json"
rm -r "$output/roles"
sums=$(cd "$output" && find . -maxdepth 1 -type f -printf '%f\0' | sort -z | xargs -0 sha256sum)
printf '%s\n' "$sums" > "$output/SHA256SUMS"
printf 'Telemetry study kit for %s (run %s, attempt %s), image %s\n' "$source_sha" "$GITHUB_RUN_ID" "$GITHUB_RUN_ATTEMPT" "$image_id"
