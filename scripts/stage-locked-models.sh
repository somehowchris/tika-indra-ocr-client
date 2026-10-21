#!/usr/bin/env bash
# Copy exactly the model files named in backend/models.lock.json from a fetched
# snapshot into a new directory and verify every size and SHA-256. Anything else
# in the snapshot, such as the fetch lock, stays behind. Prints the new
# revision directory.
set -euo pipefail
export LC_ALL=C
fail() { printf 'stage locked models: %s\n' "$*" >&2; exit 1; }
[[ $# = 2 ]] || fail 'usage: stage-locked-models.sh FETCHED_REVISION_DIRECTORY NEW_PARENT'
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
lock=$root/backend/models.lock.json
source=$(realpath -e -- "$1")
parent=$(realpath -m -- "$2")
revision=$(jq -er '.revision' "$lock")
[[ $(basename -- "$source") = "$revision" ]] || fail "the source must be the $revision snapshot"
[[ ! -e $parent && ! -L $parent ]] || fail 'the output must not already exist'
target=$parent/$revision
mkdir -p -- "$target"
sums=$parent/.locked.sha256
: > "$sums"
count=0
while IFS=$'\t' read -r name size digest; do
    [[ $name =~ ^[A-Za-z0-9._-]+(/[A-Za-z0-9._-]+)*$ && /$name/ != *'/../'* ]] || fail "unsafe model name: $name"
    [[ -f $source/$name && ! -L $source/$name ]] || fail "missing model file: $name"
    mkdir -p -- "$(dirname -- "$target/$name")"
    install -m 0444 -- "$source/$name" "$target/$name"
    [[ $(stat --format=%s -- "$target/$name") = "$size" ]] || fail "size mismatch: $name"
    printf '%s  %s\n' "$digest" "$target/$name" >> "$sums"
    count=$((count + 1))
done < <(jq -r '.files[] | [.name, (.size | tostring), .sha256] | @tsv' "$lock")
[[ $count = "$(jq -er '.files | length' "$lock")" && $count -gt 0 ]] || fail 'the lock lists no files'
sha256sum --check --strict --quiet "$sums" || fail 'a staged model file does not match its locked SHA-256'
rm -- "$sums"
printf '%s\n' "$target"
