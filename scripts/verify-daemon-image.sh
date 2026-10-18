#!/usr/bin/env bash
# Verify daemon-local OCI metadata. Never equate root, manifest and config IDs.
# Moby b26fb77d3e7b01f9d21b89539c1ab428f5ca431f:
# daemon/containerd/{image_inspect,image_exporter}.go and daemon/images/image_inspect.go.
# Only Docker's containerd image store supplies this helper's required root graph.
# The json wrapper accepts jq programs; their $variables intentionally stay literal.
# shellcheck disable=SC2016
set -euo pipefail
export LC_ALL=C
umask 077

fail() { printf 'daemon image verification: %s\n' "$*" >&2; exit 1; }
[[ $# -ge 1 ]] || fail 'expected resolve, verify or bind'
mode=$1
shift
case "$mode:$#" in
    resolve:2) reference=$1; destination=$2;;
    verify:4) archive=$1; root_input=$2; platform_input=$3; destination=$4;;
    bind:3) resolution=$1; container_input=$2; destination=$3;;
    *) fail 'usage: verify-daemon-image.sh resolve IMAGE NEW_OUTPUT | verify ARCHIVE ROOT_INSPECT PLATFORM_INSPECT NEW_OUTPUT | bind RESOLUTION_DIR CONTAINER_INSPECT NEW_OUTPUT';;
esac

# Each command has a finite bound, also charged against one 900-second budget.
# These are evidence/import preparation limits, not inference resource claims.
began=$SECONDS
bounded() {
    local maximum=$1 remaining=$((900 - (SECONDS - began)))
    shift
    (( remaining > 0 )) || fail 'overall 900-second evidence deadline exceeded'
    if (( maximum > remaining )); then maximum=$remaining; fi
    timeout --signal=TERM --kill-after=5s "${maximum}s" "$@"
}

regular() {
    local file=$1 maximum=$2 size
    [[ -f "$file" && ! -L "$file" ]] || fail "expected a regular non-symlink file: $file"
    size=$(stat --format=%s -- "$file")
    if [[ ! "$size" =~ ^[0-9]{1,11}$ ]] || (( size == 0 || size > maximum )); then
        fail "file exceeds its declared byte bound: $file"
    fi
}

output=$(realpath -m -- "$destination")
[[ ! -e "$output" && ! -L "$output" ]] || fail 'output must not already exist'
mkdir -p -- "$(dirname -- "$output")"
mkdir -m 0700 -- "$output"
printf 'stage\texit_status\n' > "$output/steps.tsv"

finish() {
    local status=$?
    trap - EXIT
    # resolve owns this sole large intermediate. verify never removes its
    # caller-owned archive. Keep raw metadata and the archive/partial hash.
    if [[ "$mode" = resolve && ( -e "$output/image.tar" || -L "$output/image.tar" ) ]]; then
        if [[ -f "$output/image.tar" && ! -L "$output/image.tar" ]]; then
            stat --format=%s -- "$output/image.tar" > "$output/removed-archive-bytes.txt" || status=1
            if [[ ! -s "$output/archive.sha256" ]]; then
                timeout --signal=TERM --kill-after=5s 30s sha256sum -- "$output/image.tar" \
                    > "$output/partial-archive.sha256" 2> "$output/partial-archive.stderr" || status=1
            fi
        fi
        rm -f -- "$output/image.tar" || status=1
    fi
    printf '%s\n' "$status" > "$output/exit-status.txt"
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

capture() {
    local stage=$1 limit=$2 kib=$3 status=0
    shift 3
    (ulimit -f "$kib"; bounded "$limit" "$@") < /dev/null \
        > "$output/$stage" 2> "$output/$stage.stderr" || status=$?
    printf '%s\t%s\n' "$stage" "$status" >> "$output/steps.tsv"
    (( status == 0 )) || fail "$stage command failed with status $status; retained stderr has details"
}

snapshot() {
    regular "$1" 1048576
    capture "$2" 15 1024 cat -- "$1"
    regular "$output/$2" 1048576
}

# Exactly one JSON value per evidence file; jq's usual stream-of-values mode
# must not let a second unrelated truthy value turn malformed input into success.
json() {
    local file=$1 filter=$2
    shift 2
    regular "$file" 1048576
    bounded 15 jq -e -s "$@" 'length == 1 and (.[0] | '"$filter"')' "$file" > /dev/null
}

descriptor() {
    json "$1" '
        type == "object" and (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
        (.size | type == "number" and isfinite and floor == . and . > 0 and . <= 1048576) and
        (.mediaType | type == "string")
    ' || fail "invalid or oversized descriptor: $1"
}

blob() {
    local file=$1 desc=$2 expected digest actual
    descriptor "$desc"
    regular "$file" 1048576
    expected=$(jq -er '.size' "$desc")
    digest=$(jq -er '.digest' "$desc")
    [[ $(stat --format=%s -- "$file") = "$expected" ]] || fail "descriptor byte length differs: $file"
    actual=$(bounded 15 sha256sum -- "$file")
    [[ ${actual%% *} = "${digest#sha256:}" ]] || fail "descriptor SHA-256 differs: $file"
}

root_metadata() {
    local source=$1
    for inspect in root platform; do
        json "$source/$inspect-inspect.json" '
            type == "array" and length == 1 and (.[0] |
                (.Id | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
                .Os == "linux" and .Architecture == "amd64" and (.Variant // "") == "")
        ' || fail "invalid single linux/amd64 $inspect inspection"
        json "$source/$inspect-inspect.json" '.[0].Descriptor | type == "object"' \
            || fail 'classic/Descriptor-absent image store cannot prove the original OCI root graph'
    done
    jq '.[0].Descriptor' "$source/root-inspect.json" > "$output/root-descriptor.json"
    descriptor "$output/root-descriptor.json"
    root_digest=$(jq -er '.digest' "$output/root-descriptor.json")
    root_size=$(jq -er '.size' "$output/root-descriptor.json")
    root_media_type=$(jq -er '.mediaType' "$output/root-descriptor.json")
    json "$source/root-inspect.json" '.[0].Id == $root' --arg root "$root_digest" \
        || fail 'root inspection Id and Descriptor disagree'
    case "$root_media_type" in
        application/vnd.oci.image.index.v1+json|application/vnd.docker.distribution.manifest.list.v2+json|\
        application/vnd.oci.image.manifest.v1+json|application/vnd.docker.distribution.manifest.v2+json) ;;
        *) fail 'unsupported root media type';;
    esac
    json "$source/oci-layout.json" 'type == "object" and .imageLayoutVersion == "1.0.0"' \
        || fail 'OCI layout version is absent or unsupported'
    json "$source/layout-index.json" '
        type == "object" and .schemaVersion == 2 and
        (.mediaType // "application/vnd.oci.image.index.v1+json") == "application/vnd.oci.image.index.v1+json" and
        (.manifests | type == "array" and length > 0 and length <= 64) and
        all(.manifests[];
            (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
            (.size | type == "number" and isfinite and floor == . and . > 0 and . <= 1048576)) and
        ([.manifests[].digest] | length == (unique | length)) and
        ([.manifests[] | select(.digest == $root)] | length == 1 and
            .[0].size == $size and .[0].mediaType == $media)
    ' --arg root "$root_digest" --argjson size "$root_size" --arg media "$root_media_type" \
        || fail 'exported layout does not identify exactly the inspected root descriptor'
}

platform_metadata() {
    local source=$1
    blob "$source/root.json" "$output/root-descriptor.json"
    json "$source/root.json" 'type == "object" and .schemaVersion == 2 and .mediaType == $media' \
        --arg media "$root_media_type" || fail 'root JSON schema/media type differs'
    case "$root_media_type" in
        application/vnd.oci.image.index.v1+json|application/vnd.docker.distribution.manifest.list.v2+json)
            json "$source/root.json" '
                (.manifests | type == "array" and length > 0 and length <= 64) and
                all(.manifests[];
                    (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
                    (.size | type == "number" and isfinite and floor == . and . > 0 and . <= 1048576)) and
                ([.manifests[].digest] | length == (unique | length)) and
                ([.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64")] |
                    length == 1 and (.[0].platform.variant // "") == "")
            ' || fail 'root index must select exactly one unambiguous linux/amd64 manifest'
            jq '.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64")' \
                "$source/root.json" > "$output/platform-descriptor.json";;
        *) cp -- "$output/root-descriptor.json" "$output/platform-descriptor.json";;
    esac
    descriptor "$output/platform-descriptor.json"
    platform_digest=$(jq -er '.digest' "$output/platform-descriptor.json")
    platform_size=$(jq -er '.size' "$output/platform-descriptor.json")
    platform_media_type=$(jq -er '.mediaType' "$output/platform-descriptor.json")
    case "$platform_media_type" in
        application/vnd.oci.image.manifest.v1+json|application/vnd.docker.distribution.manifest.v2+json) ;;
        *) fail 'selected target is not a supported image manifest';;
    esac
    json "$source/platform-inspect.json" '
        .[0] | .Id == $digest and .Descriptor.digest == $digest and
        .Descriptor.size == $size and .Descriptor.mediaType == $media and
        (if .Descriptor.platform == null then true else
            .Descriptor.platform.os == "linux" and .Descriptor.platform.architecture == "amd64" and
            (.Descriptor.platform.variant // "") == "" end)
    ' --arg digest "$platform_digest" --argjson size "$platform_size" --arg media "$platform_media_type" \
        || fail 'platform-qualified inspection disagrees with the verified root graph'
}

config_metadata() {
    local source=$1
    blob "$source/platform.json" "$output/platform-descriptor.json"
    json "$source/platform.json" '
        type == "object" and .schemaVersion == 2 and .mediaType == $media and
        (.layers | type == "array" and length <= 1024) and
        all(.layers[];
            (.digest | type == "string" and test("^sha256:[0-9a-f]{64}$")) and
            (.size | type == "number" and isfinite and floor == . and . >= 0 and . <= 8589934592)) and
        (.config.mediaType == "application/vnd.oci.image.config.v1+json" or
            .config.mediaType == "application/vnd.docker.container.image.v1+json")
    ' --arg media "$platform_media_type" || fail 'platform manifest schema/config/layer descriptors are invalid'
    jq '.config' "$source/platform.json" > "$output/config-descriptor.json"
    descriptor "$output/config-descriptor.json"
    config_digest=$(jq -er '.digest' "$output/config-descriptor.json")
    config_size=$(jq -er '.size' "$output/config-descriptor.json")
}

validate_config() {
    local source=$1
    blob "$source/config.json" "$output/config-descriptor.json"
    json "$source/config.json" '
        type == "object" and .os == "linux" and .architecture == "amd64" and (.variant // "") == "" and
        .rootfs.type == "layers" and (.rootfs.diff_ids | type == "array" and length <= 1024 and
            all(.[]; type == "string" and test("^sha256:[0-9a-f]{64}$")))
    ' || fail 'raw image configuration does not describe linux/amd64 layers'
    for inspect in root platform; do
        json "$source/$inspect-inspect.json" '
            .[0] | .RootFS.Type == $config[0].rootfs.type and .RootFS.Layers == $config[0].rootfs.diff_ids and
            (if .Descriptor.annotations["config.digest"] == null then true else
                .Descriptor.annotations["config.digest"] == $digest end)
        ' --slurpfile config "$source/config.json" --arg digest "$config_digest" \
            || fail "$inspect inspection does not bind the verified config/layer identities"
    done
    json "$source/config.json" '(.rootfs.diff_ids | length) == ($manifest[0].layers | length)' \
        --slurpfile manifest "$source/platform.json" || fail 'config and manifest layer counts disagree'
}

write_identity() {
    jq -n --arg mode "$mode" --arg root "$root_digest" --argjson root_size "$root_size" --arg root_media "$root_media_type" \
        --arg platform "$platform_digest" --argjson platform_size "$platform_size" --arg platform_media "$platform_media_type" \
        --arg config "$config_digest" --argjson config_size "$config_size" '
        {schema_version:1,passed:true,store:"containerd",mode:$mode,image_id:$root,root_digest:$root,
         root_size:$root_size,root_media_type:$root_media,platform_digest:$platform,platform_size:$platform_size,
         platform_media_type:$platform_media,config_digest:$config,config_size:$config_size,os:"linux",architecture:"amd64",
         scope:"Raw OCI metadata sizes/SHA-256 and daemon root/selected-manifest/config binding. Layer payloads and provenance are not independently authenticated by this helper."}
    ' > "$output/identity.json"
}

archive_member() {
    local name=$1 target=$2 count
    [[ "$name" = index.json || "$name" = oci-layout || "$name" =~ ^blobs/sha256/[0-9a-f]{64}$ ]] \
        || fail 'invalid internal OCI member name'
    count=$(awk -v name="$name" '$0 == name {n++} END {print n+0}' "$output/archive-list.txt")
    [[ "$count" = 1 ]] || fail "missing or duplicate OCI archive member: $name"
    capture "$target.header" 180 1024 tar --list --verbose --numeric-owner --full-time \
        --quoting-style=escape --file "$archive" -- "$name"
    awk 'NR == 1 && substr($0,1,1) == "-" {regular=1} END {exit !(NR==1 && regular)}' \
        "$output/$target.header" || fail "OCI metadata must be a regular tar entry: $name"
    capture "$target" 180 1024 tar --extract --to-stdout --occurrence=1 --file "$archive" -- "$name"
    regular "$output/$target" 1048576
}

if [[ "$mode" = bind ]]; then
    [[ -d "$resolution" && ! -L "$resolution" ]] || fail 'resolution must be a regular directory'
    resolution=$(realpath -- "$resolution")
    root_metadata "$resolution"
    platform_metadata "$resolution"
    config_metadata "$resolution"
    validate_config "$resolution"
    snapshot "$container_input" container-inspect.json
    json "$output/container-inspect.json" '
        type == "array" and length == 1 and (.[0] |
            (.Id | type == "string" and test("^[0-9a-f]{64}$")) and .Image == $root and .Platform == "linux" and
            .ImageManifestDescriptor.digest == $platform and .ImageManifestDescriptor.size == $size and
            .ImageManifestDescriptor.mediaType == $media and
            (if .ImageManifestDescriptor.annotations["config.digest"] == null then true else
                .ImageManifestDescriptor.annotations["config.digest"] == $config end) and
            .ImageManifestDescriptor.platform.os == "linux" and .ImageManifestDescriptor.platform.architecture == "amd64" and
            (.ImageManifestDescriptor.platform.variant // "") == "")
    ' --arg root "$root_digest" --arg platform "$platform_digest" --argjson size "$platform_size" \
        --arg media "$platform_media_type" --arg config "$config_digest" \
        || fail 'container root or selected ImageManifestDescriptor differs'
    write_identity
    jq --slurpfile container "$output/container-inspect.json" '. + {
        container_id:$container[0][0].Id,container_image:$container[0][0].Image,
        container_manifest_digest:$container[0][0].ImageManifestDescriptor.digest}' \
        "$output/identity.json" > "$output/result.json"
    exit 0
fi

if [[ "$mode" = resolve ]]; then
    [[ "$reference" =~ ^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,511}$ ]] || fail 'invalid bounded Docker image reference'
    capture root-inspect.json 30 1024 docker image inspect -- "$reference"
    json "$output/root-inspect.json" 'type == "array" and length == 1 and
        (.[0].Id | type == "string" and test("^sha256:[0-9a-f]{64}$"))' || fail 'cannot resolve one immutable image Id'
    json "$output/root-inspect.json" '.[0].Descriptor | type == "object"' \
        || fail 'classic/Descriptor-absent image store cannot prove the original OCI root graph'
    root_id=$(jq -er '.[0].Id' "$output/root-inspect.json")
    capture platform-inspect.json 30 1024 docker image inspect --platform linux/amd64 -- "$root_id"
    # No --platform: selecting it would discard the enclosing index being proved.
    # File size is additionally checked after the daemon has closed the stream.
    capture image.tar 300 8388608 docker image save -- "$root_id"
    archive="$output/image.tar"
else
    snapshot "$root_input" root-inspect.json
    snapshot "$platform_input" platform-inspect.json
fi
regular "$archive" 8589934592
archive=$(realpath -- "$archive")
capture archive.sha256 180 1024 sha256sum -- "$archive"
archive_hash=$(cut -d' ' -f1 "$output/archive.sha256")
capture archive-list.txt 180 2048 tar --list --quoting-style=escape --file "$archive"
awk '
    NR > 8192 {exit 1}
    !($0 == "index.json" || $0 == "oci-layout" || $0 == "manifest.json" || $0 == "repositories" ||
      $0 == "blobs/" || $0 == "blobs/sha256/" || $0 ~ /^blobs\/sha256\/[0-9a-f]+$/) {exit 1}
    /^blobs\/sha256\/[0-9a-f]+$/ && length($0) != 77 {exit 1}
    {if (seen[$0]++) exit 1}
    END {if (NR == 0) exit 1}
' "$output/archive-list.txt" || fail 'unexpected, duplicate or excessive archive member names'
archive_member oci-layout oci-layout.json
archive_member index.json layout-index.json
root_metadata "$output"
archive_member "blobs/sha256/${root_digest#sha256:}" root.json
platform_metadata "$output"
if [[ "$root_digest" = "$platform_digest" ]]; then
    ln -- "$output/root.json" "$output/platform.json"
else
    archive_member "blobs/sha256/${platform_digest#sha256:}" platform.json
fi
config_metadata "$output"
archive_member "blobs/sha256/${config_digest#sha256:}" config.json
validate_config "$output"
capture archive-after.sha256 180 1024 sha256sum -- "$archive"
[[ $(cut -d' ' -f1 "$output/archive-after.sha256") = "$archive_hash" ]] || fail 'archive changed while verifying'
write_identity
jq --arg hash "$archive_hash" --argjson bytes "$(stat --format=%s -- "$archive")" \
    '. + {archive_sha256:$hash,archive_bytes:$bytes}' "$output/identity.json" > "$output/result.json"
