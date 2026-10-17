#!/usr/bin/env bash
# Control-flow regression tests only; actual signatures are verified in release CI.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/bin"
cat > "$temporary/bin/cosign" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$1" >> "$SIGN_TEST_CALLS"
if [[ ${SIGN_TEST_FAIL:-} == "$1" ]]; then
  printf 'deliberate test failure\n' >&2
  exit 17
fi
# A tool attempting to read stdin must see EOF, not consume the digest list.
if IFS= read -r unexpected; then
  printf 'signing subprocess inherited the digest-list input\n' >&2
  exit 18
fi
head -c 524288 /dev/zero | tr '\0' x
SH
cat > "$temporary/bin/syft" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'sbom\n' >> "$SIGN_TEST_CALLS"
[[ $3 == spdx-json=* ]]
printf '{"test_double":true}\n' > "${3#spdx-json=}"
SH
chmod +x "$temporary/bin/cosign" "$temporary/bin/syft"
export PATH="$temporary/bin:$PATH"
export SIGN_TEST_CALLS="$temporary/calls"
for name in bridge tika backend tesseract; do
  printf 'ghcr.io/example/indra-%s@sha256:%064d\n' "$name" 0
done > "$temporary/digests"
bash "$root/scripts/sign-images.sh" "$temporary/digests" "$temporary/pass" \
  'https://github.com/example/repo/.github/workflows/release.yml@refs/heads/main' \
  > "$temporary/console"
[[ $(wc -l < "$SIGN_TEST_CALLS") == 20 ]]
[[ $(wc -c < "$temporary/console") -lt 4096 ]]
for name in bridge tika backend tesseract; do
  [[ $(wc -l < "$temporary/pass/indra-$name/steps.tsv") == 5 ]]
  [[ $(wc -c < "$temporary/pass/indra-$name/verify-attestation.stdout") == 524288 ]]
  [[ -s $temporary/pass/indra-$name/sbom.spdx.json ]]
done
export SIGN_TEST_FAIL=verify
status=0
bash "$root/scripts/sign-images.sh" "$temporary/digests" "$temporary/fail" \
  'https://github.com/example/repo/.github/workflows/release.yml@refs/heads/main' \
  > "$temporary/fail-console" 2> "$temporary/fail-stderr" || status=$?
[[ $status == 17 ]]
[[ ! -e $temporary/fail/indra-bridge/verify-attestation.stdout ]]
[[ ! -d $temporary/fail/indra-tika ]]
[[ $(tail -n 1 "$temporary/fail/indra-bridge/steps.tsv" | cut -f 2) == 17 ]]
printf 'ghcr.io/example/indra-bridge:latest\n' > "$temporary/invalid"
status=0
bash "$root/scripts/sign-images.sh" "$temporary/invalid" "$temporary/invalid-result" \
  'https://github.com/example/repo/.github/workflows/release.yml@refs/heads/main' \
  > /dev/null 2> /dev/null || status=$?
[[ $status == 2 && ! -d $temporary/invalid-result ]]
printf 'Signing orchestration: bounded console, preserved output, EOF stdin, failure propagation and immutable input checks passed.\n'
