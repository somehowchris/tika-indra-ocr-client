#!/usr/bin/env bash
# Offline fixture for scripts/stage-locked-models.sh: exact lock-listed copy,
# stray files left behind, and corrupted or missing files rejected.
set -euo pipefail
export LC_ALL=C
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d)
trap 'chmod -R u+w "$work"; rm -rf -- "$work"' EXIT
revision=0123456789abcdef0123456789abcdef01234567
mkdir -p "$work/root/scripts" "$work/root/backend" "$work/fetch/$revision/math"
cp "$repository/scripts/stage-locked-models.sh" "$work/root/scripts/"
printf 'detector' > "$work/fetch/$revision/det.onnx"
printf 'decoder' > "$work/fetch/$revision/math/dec.onnx"
: > "$work/fetch/$revision/.fetch.lock"
chmod 0000 "$work/fetch/$revision/.fetch.lock"
entry() { jq -n --arg name "$1" --arg sha "$(sha256sum -- "$work/fetch/$revision/$1" | cut -d' ' -f1)" \
    --argjson size "$(stat --format=%s -- "$work/fetch/$revision/$1")" '{name: $name, size: $size, sha256: $sha}'; }
jq -n --arg revision "$revision" --argjson det "$(entry det.onnx)" --argjson dec "$(entry math/dec.onnx)" \
    '{revision: $revision, files: [$det, $dec]}' > "$work/root/backend/models.lock.json"
stage() { bash "$work/root/scripts/stage-locked-models.sh" "$work/fetch/$revision" "$work/$1"; }

staged=$(stage ok)
[[ $staged = "$work/ok/$revision" ]]
[[ $(cd "$staged" && find . -type f | sort | tr '\n' ' ') = './det.onnx ./math/dec.onnx ' ]]
[[ $(stat --format=%a -- "$staged/det.onnx") = 444 ]]
cmp "$staged/math/dec.onnx" "$work/fetch/$revision/math/dec.onnx"

printf 'tampered' > "$work/fetch/$revision/det.onnx"
if stage corrupt 2> /dev/null; then echo 'a corrupted file was staged' >&2; exit 1; fi
rm "$work/fetch/$revision/det.onnx"
if stage missing 2> /dev/null; then echo 'a missing file was not rejected' >&2; exit 1; fi
if stage ok 2> /dev/null; then echo 'an existing output was overwritten' >&2; exit 1; fi
echo 'Locked model staging fixture passed.'
