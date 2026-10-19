#!/usr/bin/env bash
# Exact prospective patch of one pinned diagnostic dependency; never OCR input.
set -euo pipefail
set -o noclobber
export LC_ALL=C
[[ $# = 3 ]] || { echo 'usage: patch-inferno-profile.sh SOURCE ORIGINAL_TAR EVIDENCE_DIRECTORY' >&2; exit 1; }
source_dir=$(realpath -e -- "$1")
original=$(realpath -e -- "$2")
evidence=$(realpath -e -- "$3")
patch_file=$(realpath -e -- "$(dirname -- "${BASH_SOURCE[0]}")/patches/inferno-anonymous-namespace.patch")
# Cargo source itself is expected; owning manifests above it are forbidden.
ancestor=$source_dir
while [[ $ancestor != / ]]; do
    ancestor=$(dirname -- "$ancestor")
    [[ ! -e $ancestor/Cargo.toml && ! -L $ancestor/Cargo.toml ]] || {
        echo 'Inferno source has an owning Cargo manifest' >&2; exit 1;
    }
done
[[ $evidence != "$source_dir" && $evidence != "$source_dir/"* ]]
printf '%s  %s\n' \
  1b31468bfd351e0bce7caf2b36f062e7915913c311276d56ac7d15b9571069a4 "$original" \
  5fc2ac673f037e067bfea81dc4a4c66683d7997eb60384d21f78bf16e47638c7 "$source_dir/src/collapse/perf.rs" \
  2007624458af1c16455cff841f583230beff205146f182f19225bb81075adb51 "$source_dir/Cargo.lock" \
  04ae4d49bc2049f0ff6525d8b74a40bffd1ef04153eac80c794c29496496926f "$patch_file" | sha256sum --check --strict
git -C "$source_dir" apply --check --whitespace=error "$patch_file"
git -C "$source_dir" apply --whitespace=error "$patch_file"
printf '%s  %s\n' \
  971926929997414ceb49d9084a00d93e67e8a30b222c92a9517e69516fd2c679 "$source_dir/src/collapse/perf.rs" \
  2007624458af1c16455cff841f583230beff205146f182f19225bb81075adb51 "$source_dir/Cargo.lock" | sha256sum --check --strict
cat -- "$patch_file" > "$evidence/inferno-anonymous-namespace.patch"
[[ ! -e $evidence/inferno-patched-source.tar && ! -L $evidence/inferno-patched-source.tar ]]
tar --create --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner --format=gnu \
    --file "$evidence/inferno-patched-source.tar" --directory "$source_dir" .
printf '%s\n' \
  'upstream_commit=929be68e19a27da062623c6da5255e8b96faebbc' \
  'upstream_perf_rs_sha256=5fc2ac673f037e067bfea81dc4a4c66683d7997eb60384d21f78bf16e47638c7' \
  'patched_perf_rs_sha256=971926929997414ceb49d9084a00d93e67e8a30b222c92a9517e69516fd2c679' \
  'scope=preserve qualified C++ anonymous namespace symbols; no empty-stack synthesis or sampling-rule change' \
  > "$evidence/inferno-patch-provenance.txt"
sha256sum "$original" "$evidence/inferno-patched-source.tar" \
    "$evidence/inferno-anonymous-namespace.patch" "$evidence/inferno-patch-provenance.txt" \
    > "$evidence/inferno-patch.sha256"
