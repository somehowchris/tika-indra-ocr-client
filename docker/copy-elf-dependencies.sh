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

# Record the owning Debian package of every copied file in the distroless
# status.d convention, so an SBOM scanner can name the shipped closure.
# dpkg-query -S also prints dpkg-divert records ("diversion by libc6 from:
# ...") for diverted paths such as the dynamic loader; they name no owner and
# are skipped. Several owners print as "a:arch, b: path"; the first is kept
# without its architecture qualifier.
owner() {
    dpkg-query -S "$1" 2>/dev/null | awk -F': ' '
        /^diversion by / { next }
        { split($1, owners, ", "); sub(/:.*/, "", owners[1]); print owners[1]; exit }'
}

record_package() {
    # Debian 13 packages list files under /usr; ldd may print the /lib alias.
    package=$(owner "$1")
    if [ -z "$package" ]; then
        package=$(owner "$(readlink -f "$1")")
    fi
    if [ -z "$package" ]; then
        printf 'no Debian package owns %s\n' "$1" >&2
        exit 1
    fi
    status="$output_root/var/lib/dpkg/status.d/$package"
    if [ ! -f "$status" ]; then
        install -d -m 0755 "$output_root/var/lib/dpkg/status.d"
        dpkg-query -s "$package" > "$status"
    fi
}

record_package "$executable"
while IFS= read -r library; do
    # Resolve Debian's /lib -> /usr/lib directory, retaining each SONAME.
    # Resolving the entire file path would discard names such as libtesseract.so.5.
    directory=$(readlink -f "$(dirname "$library")")
    name=$(basename "$library")
    install -D -m 0755 "$library" "$output_root$directory/$name"
    record_package "$library"
done < "$dependencies"
