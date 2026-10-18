#!/usr/bin/env bash
# Run in the pinned GNU builder after linking. Never execute this in a service.
set -euo pipefail
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# = 3 ]] || fail 'usage: collect-runtime-materials.sh native|gateway OUTPUT BINARY'
kind=$1
output=$2
binary=$3
[[ "$kind" = native || "$kind" = gateway ]] || fail 'unknown runtime kind'
[[ "$output" = /* && "$binary" = /* && -f "$binary" && ! -L "$binary" ]] || fail 'absolute output and regular binary required'
[[ ! -e "$output" ]] || fail 'refusing to replace a material bundle'
mkdir -p "$output/glibc-source" "$output/licenses/rust" /app/.cargo

# Signed APT metadata supplied these sources. Check the installed version again,
# then every archive size and SHA-256 recorded in its exact source-control file.
version=$(dpkg-query -W -f='${source:Version}' libc6-dev)
match_count=0
for dsc in /glibc-source/*.dsc; do
    [[ -f "$dsc" && ! -L "$dsc" ]] || fail 'glibc source control is missing'
    if grep -qx 'Source: glibc' "$dsc" && grep -Fxq "Version: $version" "$dsc"; then
        match_count=$((match_count + 1))
        cp -- "$dsc" "$output/glibc-source/"
        awk '/^Checksums-Sha256:/ { inside=1; next } inside && /^[^ ]/ { exit } inside { print }' "$dsc" > "$output/glibc-archives.txt"
    fi
done
[[ "$match_count" = 1 && -s "$output/glibc-archives.txt" ]] || fail 'glibc source version does not match installed libc6-dev'
archive_count=0
while read -r hash bytes name; do
    [[ "$hash" =~ ^[[:xdigit:]]{64}$ && "$bytes" =~ ^[0-9]+$ && "$name" =~ ^[A-Za-z0-9][A-Za-z0-9._+~:-]*$ ]] || fail 'invalid source archive record'
    source="/glibc-source/$name"
    [[ -f "$source" && ! -L "$source" ]] || fail 'source archive is not a regular file'
    [[ $(stat --format=%s "$source") = "$bytes" ]] || fail 'source archive length differs'
    printf '%s  %s\n' "$hash" "$source" | sha256sum --check --status
    cp -- "$source" "$output/glibc-source/"
    archive_count=$((archive_count + 1))
done < "$output/glibc-archives.txt"
[[ "$archive_count" -ge 2 ]] || fail 'upstream and Debian glibc sources are both required'

for notice in /usr/share/doc/libc6/copyright /usr/share/doc/libgcc-s1/copyright /usr/share/doc/libstdc++6/copyright; do
    [[ -f "$notice" ]] || fail "missing system notice: $notice"
    package=$(basename "$(dirname "$notice")")
    cp -L -- "$notice" "$output/licenses/$package-copyright"
done
for name in LGPL-2.1 GPL-2 GPL-3; do
    cp -- "/usr/share/common-licenses/$name" "$output/licenses/$name"
done
rust_docs="$(rustc --print sysroot)/share/doc/rust"
find "$rust_docs" -maxdepth 1 -type f \( -iname 'license*' -o -iname 'copyright*' -o -iname 'notice*' \) -exec cp -- {} "$output/licenses/rust/" \;
[[ -d "$rust_docs/licenses" ]] || fail 'Rust third-party notices are missing'
cp -a -- "$rust_docs/licenses" "$output/licenses/rust/"

# Cargo's vendored sources include both code and notices and work without registry
# access. Keep the configuration relative to the extracted source root.
cd /app
cargo vendor --locked --manifest-path runtime/Cargo.toml vendor > .cargo/config.toml
tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
    --exclude='*/target' --exclude='runtime/proofs' \
    -I 'gzip -n -6' -cf "$output/rust-source.tar.gz" \
    runtime vendor .cargo backend/models.lock.json backend/tests/data scripts docker LICENSE THIRD_PARTY_NOTICES.md licenses docs
cp -- docs/runtime-relinking.md "$output/README.md"
cp -- docs/relinking.md "$output/relinking.md"
cp -- LICENSE THIRD_PARTY_NOTICES.md "$output/"
cp -- licenses/INDRA-MODEL-LICENSE.txt "$output/licenses/"

if [[ "$kind" = native ]]; then
    cp -a -- /native/evidence "$output/native-evidence"
    # Preserve archive paths expected by ort-sys, plus installed OpenCV headers.
    # They allow a recipient to rebuild Rust and relink against modified glibc
    # without first recompiling ONNX Runtime or OpenCV.
    cd /
    find native/build/onnxruntime/Release native/opencv/lib -type f -name '*.a' -print0 > /tmp/native-material-paths
    printf '%s\0' native/opencv/include native/cargo-env.sh >> /tmp/native-material-paths
    tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
        --null -I 'gzip -n -6' -cf "$output/native-link-inputs.tar.gz" -T /tmp/native-material-paths
fi

{
    printf 'runtime_kind=%s\nglibc_source_version=%s\n' "$kind" "$version"
    printf 'scope=source-and-relink-inputs; not a claim of a modified-glibc rebuild\n'
    printf 'binary_sha256=%s\n' "$(sha256sum "$binary" | cut -d' ' -f1)"
    rustc -Vv
    gcc --version
    getconf GNU_LIBC_VERSION
} > "$output/build-identity.txt"
cd "$output"
find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS
sha256sum --check --status SHA256SUMS
