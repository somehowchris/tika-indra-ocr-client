#!/usr/bin/env bash
# docs/history.md records every archived or removed path by its file count
# and, where one tree or blob covers it, its Git ID at the archive commit, and
# names the tags that retain the archive and the cited producing revisions.
# Check each recorded ID, count and tag against Git. Needs full history and
# tags (CI's history job checks out with fetch-depth 0).
# Markdown backticks in the patterns are literal.
# shellcheck disable=SC2016
set -euo pipefail
export LC_ALL=C
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$repository"
archive=1f9d4d7b1185331189daec230fa7a34343258f4e
history=docs/history.md
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
fail() { printf '%s: %s\n' "$history" "$1" >&2; exit 1; }

git cat-file -e "$archive^{commit}" || fail "archive commit $archive is not available"

# Files under $1 at the archive commit that the delivery tree no longer tracks.
removed() {
  git ls-tree -r --name-only "$archive" -- "$1" | sort > "$work/archived"
  git ls-files -- "$1" | sort > "$work/tracked"
  comm -23 "$work/archived" "$work/tracked" | wc -l | tr -d ' '
}

row="^\\| \\[([^]]*)\\]\\(https://github\\.com/somehowchris/tika-indra-ocr-client/(tree|blob)/$archive/([^)]*)\\)"
# Packet and removed-path rows: | [path](link) note | files | `git id` |
sed -nE "s#${row}[^|]*\\| ([0-9]+) \\| \`([0-9a-f]{40})\` \\|\$#\\3 \\4 \\5#p" "$history" > "$work/ids"
# Second-restoration rows: | [path](link) | archived files | removed files |
sed -nE "s#$row \\| ([0-9]+) \\| ([0-9]+) \\|\$#\\3 \\4 \\5#p" "$history" > "$work/counts"
# Multi-file rows without one Git ID: | [`dir/{a,b}.ext`](link) note | files | — |
sed -nE "s#${row}[^|]*\\| ([0-9]+) \\| — \\|\$#\\3 \\4 \\1#p" "$history" > "$work/multi"
total=$(grep -cE "$row" "$history")
parsed=$(( $(wc -l < "$work/ids") + $(wc -l < "$work/counts") + $(wc -l < "$work/multi") ))
[[ $parsed -eq $total ]] || fail "$total archive rows, but only $parsed have a recognised shape"

while read -r path files id; do
  actual=$(git rev-parse "$archive:$path")
  [[ $actual = "$id" ]] || fail "$path is $actual at $archive, not $id"
  count=$(removed "$path")
  [[ $count = "$files" ]] || fail "$path: $files files recorded, $count removed"
done < "$work/ids"

# A multi-file row names its files as one brace list under the linked
# directory; each must exist at the archive commit and no longer be tracked.
list='^`?([A-Za-z0-9_./-]+)\{([A-Za-z0-9_.,-]+)\}([A-Za-z0-9_.-]*)`?$'
while read -r path files names; do
  [[ $names =~ $list ]] || fail "cannot expand the file list $names"
  prefix=${BASH_REMATCH[1]} suffix=${BASH_REMATCH[3]}
  IFS=, read -ra parts <<< "${BASH_REMATCH[2]}"
  : > "$work/named"
  for part in "${parts[@]}"; do
    file=$prefix$part$suffix
    [[ $file = "$path"/* ]] || fail "$file is outside $path"
    git cat-file -e "$archive:$file" 2> /dev/null || fail "$file is not at $archive"
    [[ -z $(git ls-files -- "$file") ]] || fail "$file is still tracked"
    printf '%s\n' "$file" >> "$work/named"
  done
  count=$(sort -u "$work/named" | wc -l | tr -d ' ')
  [[ $count = "$files" ]] || fail "$path: $files files recorded, $count named"
done < "$work/multi"

while read -r path archived files; do
  count=$(git ls-tree -r --name-only "$archive" -- "$path" | wc -l | tr -d ' ')
  [[ $count = "$archived" ]] || fail "$path: $archived archived files recorded, $count at $archive"
  count=$(removed "$path")
  [[ $count = "$files" ]] || fail "$path: $files removed files recorded, $count removed"
done < "$work/counts"

# Retention tags: | `tag` | `commit` | cited revisions it contains |
sed -nE 's#^\| `([A-Za-z0-9._-]+)` \| `([0-9a-f]{40})` \| (.*) \|$#\1 \2 \3#p' "$history" > "$work/tags"
grep -q "^[^ ]* $archive " "$work/tags" || fail "no tag retains the archive commit $archive"
while read -r tag commit cited; do
  actual=$(git rev-parse --verify --quiet "refs/tags/$tag^{commit}") ||
    fail "tag $tag is missing; push it to origin (full-depth checkouts fetch every tag)"
  [[ $actual = "$commit" ]] || fail "tag $tag is $actual, not $commit"
  for revision in $(grep -oE '`[0-9a-f]{7,40}`' <<< "$cited" | tr -d '`'); do
    git merge-base --is-ancestor "$revision" "$commit" || fail "tag $tag does not contain $revision"
  done
done < "$work/tags"
