#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    printf 'usage: copy-elf-dependencies EXECUTABLE OUTPUT_ROOT\n' >&2
    exit 2
fi

executable=$1
output_root=$2
ldd_output=$(mktemp)
dependencies=$(mktemp)
trap 'rm -f "$ldd_output" "$dependencies"' EXIT HUP INT TERM

ldd "$executable" > "$ldd_output"
if awk '/not found/ {missing=1} END {exit !missing}' "$ldd_output"; then
    cat "$ldd_output" >&2
    exit 1
fi
awk '/=> \// {print $3} /^[[:space:]]*\// {print $1}' "$ldd_output" > "$dependencies"
if [ ! -s "$dependencies" ]; then
    printf 'no shared dependencies found for %s\n' "$executable" >&2
    exit 1
fi

while IFS= read -r library; do
    # Resolve Debian's /lib -> /usr/lib directory, retaining each SONAME.
    # Resolving the entire file path would discard names such as libtesseract.so.5.
    directory=$(readlink -f "$(dirname "$library")")
    name=$(basename "$library")
    install -D -m 0755 "$library" "$output_root$directory/$name"
done < "$dependencies"
