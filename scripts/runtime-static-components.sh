#!/usr/bin/env bash
# Run in the pinned GNU builder after linking. A scratch image has no package
# database and Syft cannot see static archives, so this names them as SPDX 2.3
# packages with purls: the glibc and libgcc archives of every crt-static Rust
# executable and, for the native service, libstdc++, ONNX Runtime, OpenCV,
# Intel IPP ICV and OpenCV's vendored codecs, for the signed image SBOM.
set -euo pipefail
export LC_ALL=C TZ=UTC
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# = 3 || $# = 4 ]] || fail 'usage: runtime-static-components.sh native|gateway|bridge BINARY OUTPUT [NATIVE_EVIDENCE]'
kind=$1
binary=$2
output=$3
case "$kind" in
    native) [[ $# = 4 ]] || fail 'the native kind requires its native evidence directory'; evidence=$4;;
    gateway|bridge) [[ $# = 3 ]] || fail "the $kind kind links no native SDK";;
    *) fail 'unknown runtime kind';;
esac
[[ -f "$binary" && ! -L "$binary" ]] || fail 'regular binary required'
[[ ! -e "$output" ]] || fail 'refusing to replace a component SBOM'
epoch=${SOURCE_DATE_EPOCH:-0}
[[ "$epoch" =~ ^[0-9]+$ ]] || fail 'SOURCE_DATE_EPOCH must be whole seconds'
printf -v created '%(%Y-%m-%dT%H:%M:%SZ)T' "$epoch"
binary_sha256=$(sha256sum "$binary" | cut -d' ' -f1)
distro=$(awk -F= '$1 == "ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
distro_version=$(awk -F= '$1 == "VERSION_ID" { gsub(/"/, "", $2); print $2 }' /etc/os-release)
[[ "$distro" =~ ^[a-z0-9]+$ && "$distro_version" =~ ^[0-9.]+$ ]] || fail 'builder distribution is not identified'

# Every interpolated value is checked against a JSON-, purl- and CPE-safe
# character set, so the document needs no escaping. SPDXIDs carry the kind, so
# the gateway and bridge sets, which the Tika image embeds together, never collide.
packages=()
relationships=()
add_package() {
    local id=$kind-$1 name=$2 version=$3 purl=$4 location=$5 info=$6 cpe=${7:-} checksum=${8:-} references
    [[ "$*" != *[\"\\]* && ! "$*" =~ [[:cntrl:]] ]] || fail "unsafe SPDX value for $name"
    references="{\"referenceCategory\":\"PACKAGE-MANAGER\",\"referenceType\":\"purl\",\"referenceLocator\":\"$purl\"}"
    [[ -z "$cpe" ]] || references+=",{\"referenceCategory\":\"SECURITY\",\"referenceType\":\"cpe23Type\",\"referenceLocator\":\"$cpe\"}"
    [[ -z "$checksum" ]] || checksum=",\"checksums\":[{\"algorithm\":\"SHA256\",\"checksumValue\":\"$checksum\"}]"
    packages+=("{\"SPDXID\":\"SPDXRef-Static-$id\",\"name\":\"$name\",\"versionInfo\":\"$version\",\"downloadLocation\":\"$location\",\"filesAnalyzed\":false$checksum,\"licenseConcluded\":\"NOASSERTION\",\"licenseDeclared\":\"NOASSERTION\",\"copyrightText\":\"NOASSERTION\",\"sourceInfo\":\"$info\",\"externalRefs\":[$references]}")
    relationships+=("{\"spdxElementId\":\"SPDXRef-DOCUMENT\",\"relationshipType\":\"DESCRIBES\",\"relatedSpdxElement\":\"SPDXRef-Static-$id\"}")
}
purl_escape() { local value=${1//+/%2B}; printf '%s' "${value//:/%3A}"; }

# The Debian package that ships the archive the linker driver resolves. dpkg
# also prints dpkg-divert records ("diversion by ..."), which name no owner.
debian_archive() {
    local id=$1 archive=$2 info=$3 path owner name version source source_version architecture upstream
    path=$(cc -print-file-name="$archive")
    [[ "$path" = /* && -f "$path" ]] || fail "the linker driver finds no $archive"
    path=$(realpath -- "$path")
    owner=$(dpkg-query -S "$path" 2>/dev/null | awk -F': ' '/^diversion by / { next } { split($1, owners, ", "); print owners[1]; exit }')
    [[ "$owner" =~ ^[a-z0-9][a-z0-9+.-]+(:[a-z0-9]+)?$ ]] || fail "no Debian package ships $path"
    IFS=$'\t' read -r name version source source_version architecture < <(dpkg-query -W \
        -f='${Package}\t${Version}\t${source:Package}\t${source:Version}\t${Architecture}\n' "$owner") \
        || fail "dpkg has no record of $owner"
    [[ "$name" =~ ^[a-z0-9][a-z0-9+.-]+$ && "$source" =~ ^[a-z0-9][a-z0-9+.-]+$ && "$architecture" =~ ^[a-z0-9-]+$ &&
       "$version" =~ ^[0-9][0-9A-Za-z.+~:-]*$ && "$source_version" =~ ^[0-9][0-9A-Za-z.+~:-]*$ ]] \
        || fail "unexpected Debian metadata for $owner"
    upstream=$(purl_escape "$source")
    [[ "$source_version" = "$version" ]] || upstream+="%40$(purl_escape "$source_version")"
    add_package "$id" "$name" "$version" \
        "pkg:deb/$distro/$(purl_escape "$name")@$(purl_escape "$version")?arch=$architecture&distro=$distro-$distro_version&upstream=$upstream" \
        NOASSERTION "$info, shipped in the builder by this Debian package"
}

debian_archive glibc libc.a 'libc.a and the other glibc archives of the crt-static link'
debian_archive libgcc libgcc_eh.a 'libgcc.a and libgcc_eh.a of the crt-static link'
if [[ "$kind" = native ]]; then
    debian_archive libstdcxx libstdc++.a 'libstdc++.a, linked statically for ONNX Runtime and OpenCV'
    for name in native-build-tools.lock vendored-components.tsv; do
        [[ -f "$evidence/$name" && ! -L "$evidence/$name" ]] || fail "native evidence lacks $name"
    done
    # shellcheck source=native-build-tools.lock
    source "$evidence/native-build-tools.lock"
    github_source() {
        local id=$1 repository=$2 revision=$3 info=$4 path
        [[ "$revision" =~ ^[0-9a-f]{40}$ && "$repository" =~ ^https://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)\.git$ ]] \
            || fail "invalid locked $id source"
        path=${BASH_REMATCH[1],,}
        add_package "$id" "${path#*/}" "$revision" "pkg:github/$path@$revision" "git+$repository@$revision" "$info"
    }
    github_source onnxruntime "$ONNXRUNTIME_REPOSITORY" "$ONNXRUNTIME_REVISION" 'static ONNX Runtime archives built from this locked revision'
    github_source opencv "$OPENCV_REPOSITORY" "$OPENCV_REVISION" \
        'static OpenCV core, flann, geometry, imgproc and imgcodecs archives built from this locked revision'
    [[ "$IPP_REVISION" =~ ^[0-9a-f]{40}$ && "$IPP_SHA256" =~ ^[0-9a-f]{64}$ &&
       "$IPP_FILENAME" =~ ^ippicv_([0-9]+\.[0-9]+\.[0-9]+)_lnx_intel64_[0-9]{8}_general\.tgz$ ]] || fail 'invalid locked IPP ICV archive'
    ipp_version=${BASH_REMATCH[1]}
    ipp_url="https://raw.githubusercontent.com/opencv/opencv_3rdparty/$IPP_REVISION/ippicv/$IPP_FILENAME"
    add_package ippicv ippicv "$ipp_version" "pkg:generic/ippicv@$ipp_version?checksum=sha256:$IPP_SHA256&download_url=$ipp_url" \
        "$ipp_url" 'Intel IPP ICV archive that OpenCV links (libippicv.a, libippiw.a)' '' "$IPP_SHA256"

    # record-native-runtime.sh read these from the version headers at build time.
    expected=(libjpeg-turbo libpng libtiff zlib)
    count=0
    {
        IFS= read -r header
        [[ "$header" = $'component\tversion\tversion_header' ]] || fail 'vendored component inventory has an unknown format'
        while IFS=$'\t' read -r component version source_header; do
            [[ "$component" = "${expected[$count]:-}" && "$version" =~ ^[0-9][0-9A-Za-z._-]*$ &&
               "$source_header" =~ ^(src|build)/opencv/3rdparty/$component/[A-Za-z0-9_.]+\.h$ ]] \
                || fail "unexpected vendored component record: $component"
            case "$component" in
                libjpeg-turbo) cpe=libjpeg-turbo:libjpeg-turbo;;
                libpng) cpe=libpng:libpng;;
                libtiff) cpe=libtiff:libtiff;;
                zlib) cpe=zlib:zlib;;
            esac
            add_package "$component" "$component" "$version" "pkg:generic/$component@$version" \
                "git+$OPENCV_REPOSITORY@$OPENCV_REVISION#3rdparty/$component" \
                "copy vendored by OpenCV and linked statically; version from $source_header" \
                "cpe:2.3:a:$cpe:$version:*:*:*:*:*:*:*"
            count=$((count + 1))
        done
    } < "$evidence/vendored-components.tsv"
    [[ "$count" = "${#expected[@]}" ]] || fail 'vendored component inventory is incomplete'
fi

set -o noclobber
{
    printf '{"spdxVersion":"SPDX-2.3","dataLicense":"CC0-1.0","SPDXID":"SPDXRef-DOCUMENT","name":"static-components-%s",\n' "$kind"
    printf '"documentNamespace":"https://github.com/somehowchris/tika-indra-ocr-client/spdx/static-components/%s/%s",\n' "$kind" "$binary_sha256"
    printf '"creationInfo":{"created":"%s","creators":["Tool: runtime-static-components.sh"]},\n' "$created"
    printf '"comment":"Static archives linked into the executable with SHA-256 %s.",\n' "$binary_sha256"
    printf '"packages":[\n'
    printf '%s\n' "${packages[@]}" | sed '$!s/$/,/'
    printf '],\n"relationships":[\n'
    printf '%s\n' "${relationships[@]}" | sed '$!s/$/,/'
    printf ']}\n'
} > "$output"
