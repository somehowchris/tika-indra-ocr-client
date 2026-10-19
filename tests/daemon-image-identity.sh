#!/usr/bin/env bash
# Offline raw-descriptor and CLI regressions. No Docker daemon or network needed.
# jq filter variables intentionally remain literal until passed to jq.
# shellcheck disable=SC2016
set -euo pipefail
export LC_ALL=C
umask 077
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
helper="$root/scripts/verify-daemon-image.sh"
[[ $# -le 1 ]] || { echo 'usage: daemon-image-identity.sh [NEW_EVIDENCE_DIRECTORY]' >&2; exit 2; }
if [[ $# = 1 ]]; then
    [[ ! -e "$1" && ! -L "$1" ]] || exit 2
    mkdir -p -- "$1"
    output=$(realpath -- "$1")
else
    output=$(mktemp -d "${TMPDIR:-$PWD}/daemon-image-tests.XXXXXX")
fi
printf 'case\texpected\tactual\n' > "$output/cases.tsv"
successes=0
failures=0
index_type=application/vnd.oci.image.index.v1+json
manifest_type=application/vnd.oci.image.manifest.v1+json
config_type=application/vnd.oci.image.config.v1+json

digest() { sha256sum -- "$1" | cut -d' ' -f1; }

descriptor() {
    local file=$1 type=$2
    jq -nc --arg digest "sha256:$(digest "$file")" --argjson size "$(stat --format=%s -- "$file")" \
        --arg type "$type" '{mediaType:$type,digest:$digest,size:$size}'
}

blob() { cp -- "$1" "$fixture/layout/blobs/sha256/$(digest "$1")"; }

fixture() {
    local name=$1 kind=$2 mutation=${3:-none}
    fixture="$output/$name"
    mkdir -p -- "$fixture/layout/blobs/sha256"
    printf '%s\n' 'A synthetic layer payload; metadata tests do not run this image.' > "$fixture/layer"
    descriptor "$fixture/layer" application/vnd.oci.image.layer.v1.tar > "$fixture/layer-descriptor.json"
    blob "$fixture/layer"
    jq -nc --arg diff "sha256:$(digest "$fixture/layer")" \
        '{os:"linux",architecture:"amd64",config:{User:"65532:65532"},rootfs:{type:"layers",diff_ids:[$diff]}}' \
        > "$fixture/config.json"
    if [[ "$mutation" = config-platform ]]; then
        jq '.architecture="arm64"' "$fixture/config.json" > "$fixture/change.json"
        mv -- "$fixture/change.json" "$fixture/config.json"
    fi
    descriptor "$fixture/config.json" "$config_type" > "$fixture/config-descriptor.json"
    blob "$fixture/config.json"
    case "$mutation" in
        config-size) jq '.size += 1' "$fixture/config-descriptor.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/config-descriptor.json";;
        config-type) jq '.mediaType="application/octet-stream"' "$fixture/config-descriptor.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/config-descriptor.json";;
    esac
    jq -nc --arg type "$manifest_type" --slurpfile config "$fixture/config-descriptor.json" \
        --slurpfile layer "$fixture/layer-descriptor.json" \
        '{schemaVersion:2,mediaType:$type,config:$config[0],layers:[$layer[0]]}' > "$fixture/platform.json"
    descriptor "$fixture/platform.json" "$manifest_type" | \
        jq -c '. + {platform:{os:"linux",architecture:"amd64"}}' > "$fixture/platform-descriptor.json"
    blob "$fixture/platform.json"
    case "$mutation" in
        manifest-size) jq '.size += 1' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        fractional-size) jq '.size += 0.5' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        oversized-descriptor) jq '.size=1048577' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        invalid-digest) jq '.digest="sha256:bad"' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        wrong-platform) jq '.platform.architecture="arm64"' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        wrong-variant) jq '.platform.variant="v3"' "$fixture/platform-descriptor.json" > "$fixture/change.json";;
        *) cp -- "$fixture/platform-descriptor.json" "$fixture/change.json";;
    esac
    mv -- "$fixture/change.json" "$fixture/platform-descriptor.json"
    if [[ "$kind" = index ]]; then
        # Preserve a second non-runnable manifest, as BuildKit provenance does.
        jq -nc '{os:"unknown",architecture:"unknown",rootfs:{type:"layers",diff_ids:[]}}' > "$fixture/attestation-config.json"
        descriptor "$fixture/attestation-config.json" "$config_type" > "$fixture/attestation-config-descriptor.json"
        blob "$fixture/attestation-config.json"
        jq -nc --arg type "$manifest_type" --slurpfile config "$fixture/attestation-config-descriptor.json" \
            '{schemaVersion:2,mediaType:$type,config:$config[0],layers:[]}' > "$fixture/attestation.json"
        blob "$fixture/attestation.json"
        descriptor "$fixture/attestation.json" "$manifest_type" | jq -c '. + {
            platform:{os:"unknown",architecture:"unknown"},annotations:{"vnd.docker.reference.type":"attestation-manifest"}}' \
            > "$fixture/attestation-descriptor.json"
        jq -nc --arg type "$index_type" --slurpfile platform "$fixture/platform-descriptor.json" \
            --slurpfile attestation "$fixture/attestation-descriptor.json" \
            '{schemaVersion:2,mediaType:$type,manifests:[$platform[0],$attestation[0]]}' > "$fixture/root.json"
        case "$mutation" in
            duplicate-platform) jq '.manifests[1].platform={os:"linux",architecture:"amd64"}' "$fixture/root.json" > "$fixture/change.json";;
            duplicate-descriptor) jq '.manifests += [.manifests[0]]' "$fixture/root.json" > "$fixture/change.json";;
            bad-root-schema) jq '.schemaVersion=3' "$fixture/root.json" > "$fixture/change.json";;
            *) cp -- "$fixture/root.json" "$fixture/change.json";;
        esac
        mv -- "$fixture/change.json" "$fixture/root.json"
        descriptor "$fixture/root.json" "$index_type" > "$fixture/root-descriptor.json"
    else
        cp -- "$fixture/platform.json" "$fixture/root.json"
        descriptor "$fixture/root.json" "$manifest_type" > "$fixture/root-descriptor.json"
    fi
    blob "$fixture/root.json"
    if [[ "$mutation" = root-size ]]; then
        jq '.size += 1' "$fixture/root-descriptor.json" > "$fixture/change.json"
        mv -- "$fixture/change.json" "$fixture/root-descriptor.json"
    fi
    jq -nc --slurpfile desc "$fixture/root-descriptor.json" '{schemaVersion:2,manifests:[$desc[0]]}' \
        > "$fixture/layout/index.json"
    printf '%s\n' '{"imageLayoutVersion":"1.0.0"}' > "$fixture/layout/oci-layout"
    jq -nc --arg config "blobs/sha256/$(digest "$fixture/config.json")" \
        --arg layer "blobs/sha256/$(digest "$fixture/layer")" \
        '[{Config:$config,RepoTags:["example:dev"],Layers:[$layer]}]' > "$fixture/layout/manifest.json"
    for item in root platform; do
        jq -nc --slurpfile desc "$fixture/$item-descriptor.json" --arg diff "sha256:$(digest "$fixture/layer")" \
            '[{Id:$desc[0].digest,Descriptor:$desc[0],Os:"linux",Architecture:"amd64",
               RootFS:{Type:"layers",Layers:[$diff]},RepoTags:["example:dev"],RepoDigests:["deliberately-irrelevant@sha256:000"]}]' \
            > "$fixture/$item-inspect.json"
    done
    jq -nc --slurpfile root "$fixture/root-descriptor.json" --slurpfile platform "$fixture/platform-descriptor.json" \
        --arg id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        '[{Id:$id,Image:$root[0].digest,Platform:"linux",ImageManifestDescriptor:$platform[0],Config:{Image:"example:dev"}}]' \
        > "$fixture/container.json"
    case "$mutation" in
        root-id) jq '.[0].Id="sha256:"+("0"*64)' "$fixture/root-inspect.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/root-inspect.json";;
        layout-root) jq '.manifests[0].digest="sha256:"+("0"*64)' "$fixture/layout/index.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/layout/index.json";;
        platform-id-config) jq --arg id "sha256:$(digest "$fixture/config.json")" '.[0].Id=$id' "$fixture/platform-inspect.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/platform-inspect.json";;
        inspection-platform-size) jq '.[0].Descriptor.size += 1' "$fixture/platform-inspect.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/platform-inspect.json";;
        classic) jq --arg id "sha256:$(digest "$fixture/config.json")" '.[0].Id=$id | del(.[0].Descriptor)' "$fixture/root-inspect.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/root-inspect.json";;
        rootfs) jq '.[0].RootFS.Layers=[]' "$fixture/platform-inspect.json" > "$fixture/change.json"; mv -- "$fixture/change.json" "$fixture/platform-inspect.json";;
        multiple-json) printf '%s\n' 'true' >> "$fixture/root-inspect.json";;
        missing-config) rm -- "$fixture/layout/blobs/sha256/$(digest "$fixture/config.json")";;
        missing-root) rm -- "$fixture/layout/blobs/sha256/$(digest "$fixture/root.json")";;
        changed-config) printf ' ' >> "$fixture/layout/blobs/sha256/$(digest "$fixture/config.json")";;
        changed-manifest) printf ' ' >> "$fixture/layout/blobs/sha256/$(digest "$fixture/platform.json")";;
        symlink-config) rm -- "$fixture/layout/blobs/sha256/$(digest "$fixture/config.json")"; ln -s -- /etc/passwd "$fixture/layout/blobs/sha256/$(digest "$fixture/config.json")";;
        hardlink-config) ln -- "$fixture/layout/blobs/sha256/$(digest "$fixture/config.json")" "$fixture/layout/blobs/sha256/0000000000000000000000000000000000000000000000000000000000000000";;
        no-legacy) rm -- "$fixture/layout/manifest.json";;
        unexpected-member) printf 'unread\n' > "$fixture/layout/unexpected";;
    esac
    (cd -- "$fixture/layout"; find . -mindepth 1 -printf '%P\0' | sort -z > "$fixture/members.nul")
    tar --create --no-recursion --null --directory "$fixture/layout" \
        --files-from "$fixture/members.nul" --file "$fixture/image.tar"
    if [[ "$mutation" = duplicate-member ]]; then
        tar --append --file "$fixture/image.tar" --directory "$fixture/layout" -- index.json
    fi
}

expect() {
    local name=$1 expectation=$2 status=0
    shift 2
    timeout --signal=TERM --kill-after=5s 60s "$@" > "$output/$name.stdout" 2> "$output/$name.stderr" || status=$?
    printf '%s\t%s\t%s\n' "$name" "$expectation" "$status" >> "$output/cases.tsv"
    if [[ "$expectation" = pass && "$status" = 0 ]] || [[ "$expectation" = reject && "$status" = 1 ]]; then
        successes=$((successes + 1))
    else
        failures=$((failures + 1))
        printf 'Unexpected result: %s expected %s, got %s\n' "$name" "$expectation" "$status" >&2
        tail -c 2048 "$output/$name.stderr" >&2
    fi
}

verify() {
    expect "$1" "$2" "$helper" verify "$fixture/image.tar" "$fixture/root-inspect.json" \
        "$fixture/platform-inspect.json" "$fixture/verified"
}

for kind in index manifest; do
    fixture "valid-$kind" "$kind"
    verify "valid-$kind" pass
    expect "bind-$kind" pass "$helper" bind "$fixture/verified" "$fixture/container.json" "$fixture/bound"
    jq -e --arg root "sha256:$(digest "$fixture/root.json")" --arg platform "sha256:$(digest "$fixture/platform.json")" \
        --arg config "sha256:$(digest "$fixture/config.json")" \
        '.passed == true and .store == "containerd" and .image_id == $root and .root_digest == $root and
         .platform_digest == $platform and .config_digest == $config and .config_digest != .platform_digest' \
        "$fixture/verified/result.json" > /dev/null
done

index_type=application/vnd.docker.distribution.manifest.list.v2+json
manifest_type=application/vnd.docker.distribution.manifest.v2+json
config_type=application/vnd.docker.container.image.v1+json
fixture valid-docker-media index
verify valid-docker-media pass
expect bind-docker-media pass "$helper" bind "$fixture/verified" "$fixture/container.json" "$fixture/bound"
index_type=application/vnd.oci.image.index.v1+json
manifest_type=application/vnd.oci.image.manifest.v1+json
config_type=application/vnd.oci.image.config.v1+json
fixture valid-without-legacy index no-legacy
verify valid-without-legacy pass

for mutation in config-size config-type config-platform manifest-size fractional-size oversized-descriptor \
    invalid-digest wrong-platform wrong-variant duplicate-platform duplicate-descriptor bad-root-schema root-size \
    root-id layout-root platform-id-config inspection-platform-size classic rootfs multiple-json missing-config \
    missing-root changed-config changed-manifest symlink-config hardlink-config unexpected-member duplicate-member; do
    fixture "$mutation" index "$mutation"
    verify "$mutation" reject
done

fixture bind-cases index
verify bind-cases pass
for mutation in wrong-root config-as-root wrong-manifest wrong-size wrong-media wrong-os wrong-architecture \
    missing-descriptor missing-platform wrong-config-annotation short-id multiple-containers; do
    case "$mutation" in
        wrong-root) filter='.[0].Image="sha256:"+("0"*64)';;
        config-as-root) filter='.[0].Image=$config';;
        wrong-manifest) filter='.[0].ImageManifestDescriptor.digest="sha256:"+("0"*64)';;
        wrong-size) filter='.[0].ImageManifestDescriptor.size += 1';;
        wrong-media) filter='.[0].ImageManifestDescriptor.mediaType="application/vnd.oci.image.index.v1+json"';;
        wrong-os) filter='.[0].ImageManifestDescriptor.platform.os="windows"';;
        wrong-architecture) filter='.[0].ImageManifestDescriptor.platform.architecture="arm64"';;
        missing-descriptor) filter='del(.[0].ImageManifestDescriptor)';;
        missing-platform) filter='del(.[0].ImageManifestDescriptor.platform)';;
        wrong-config-annotation) filter='.[0].ImageManifestDescriptor.annotations={"config.digest":("sha256:"+("0"*64))}';;
        short-id) filter='.[0].Id="abc"';;
        multiple-containers) filter='. + [.[0]]';;
    esac
    jq --arg config "sha256:$(digest "$fixture/config.json")" "$filter" "$fixture/container.json" > "$fixture/$mutation.json"
    expect "bind-$mutation" reject "$helper" bind "$fixture/verified" "$fixture/$mutation.json" "$fixture/bound-$mutation"
done
jq '.[0].Config.Image="unrelated:mutable-tag"' "$fixture/container.json" > "$fixture/moved-tag.json"
expect bind-ignores-tag pass "$helper" bind "$fixture/verified" "$fixture/moved-tag.json" "$fixture/bound-moved-tag"
printf ' ' >> "$fixture/verified/config.json"
expect bind-revalidates-raw-config reject "$helper" bind "$fixture/verified" "$fixture/container.json" "$fixture/bound-changed-config"

# A CLI double checks the actual immutable command arguments. It cannot return
# a passing container/image by accepting whatever string the implementation uses.
fixture resolve index
mkdir -- "$output/bin"
cat > "$output/bin/docker" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
jq -nc --args '$ARGS.positional' -- "$@" >> "$DAEMON_FIXTURE/commands.jsonl"
case "$*" in
    'image inspect -- example:dev') cat -- "$DAEMON_FIXTURE/root-inspect.json";;
    "image inspect --platform linux/amd64 -- $DAEMON_ROOT") cat -- "$DAEMON_FIXTURE/platform-inspect.json";;
    "image save -- $DAEMON_ROOT")
        if [[ ${DAEMON_EXPORT_ERROR:-0} = 1 ]]; then printf 'partial owned archive\n'; exit 17; fi
        cat -- "$DAEMON_FIXTURE/image.tar";;
    *) printf 'unexpected Docker arguments\n' >&2; exit 42;;
esac
SH
chmod 0755 "$output/bin/docker"
expect resolve-immutable pass env PATH="$output/bin:$PATH" DAEMON_FIXTURE="$fixture" \
    DAEMON_ROOT="sha256:$(digest "$fixture/root.json")" "$helper" resolve example:dev "$fixture/resolved"
[[ ! -e "$fixture/resolved/image.tar" && -s "$fixture/resolved/archive.sha256" ]] \
    || { echo 'resolve did not clean up only its owned archive' >&2; exit 1; }
jq -se --arg root "sha256:$(digest "$fixture/root.json")" '. == [
    ["image","inspect","--","example:dev"],
    ["image","inspect","--platform","linux/amd64","--",$root],
    ["image","save","--",$root]]' "$fixture/commands.jsonl" > /dev/null
expect resolve-export-failure reject env PATH="$output/bin:$PATH" DAEMON_FIXTURE="$fixture" DAEMON_EXPORT_ERROR=1 \
    DAEMON_ROOT="sha256:$(digest "$fixture/root.json")" "$helper" resolve example:dev "$fixture/failed-export"
[[ ! -e "$fixture/failed-export/image.tar" && -s "$fixture/failed-export/partial-archive.sha256" ]] \
    || { echo 'partial owned archive cleanup/evidence failed' >&2; exit 1; }
grep -Fxq $'image.tar\t17' "$fixture/failed-export/steps.tsv" \
    || { echo 'original export failure was lost' >&2; exit 1; }
fixture resolve-wrong-graph index layout-root
expect resolve-wrong-graph reject env PATH="$output/bin:$PATH" DAEMON_FIXTURE="$fixture" \
    DAEMON_ROOT="sha256:$(digest "$fixture/root.json")" "$helper" resolve example:dev "$fixture/failed-graph"
[[ ! -e "$fixture/failed-graph/image.tar" && -s "$fixture/failed-graph/archive.sha256" ]] \
    || { echo 'failed graph did not retain identity and clean only its owned archive' >&2; exit 1; }
fixture="$output/resolve"
expect cannot-overwrite-output reject "$helper" verify "$fixture/image.tar" "$fixture/root-inspect.json" \
    "$fixture/platform-inspect.json" "$fixture/resolved"
ln -s -- "$fixture/image.tar" "$fixture/image-link.tar"
expect rejects-archive-symlink reject "$helper" verify "$fixture/image-link.tar" "$fixture/root-inspect.json" \
    "$fixture/platform-inspect.json" "$fixture/symlink-input"
[[ -f "$fixture/image.tar" && -L "$fixture/image-link.tar" ]] \
    || { echo 'verify removed caller-owned input' >&2; exit 1; }

jq -n --argjson passed "$successes" --argjson failed "$failures" --arg helper "$(digest "$helper")" \
    '{schema_version:1,passed:$passed,failed:$failed,helper_sha256:$helper,
      scope:"Offline raw OCI metadata and command-shape fixtures; no Docker daemon, runnable image, network, lifecycle or performance claim."}' \
    > "$output/result.json"
cat -- "$output/result.json"
printf 'Evidence: %s\n' "$output"
(( failures == 0 ))
