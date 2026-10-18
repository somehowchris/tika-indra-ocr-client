#!/usr/bin/env bash
# Bind a real CRI container's immutable creation config to a verified OCI graph.
set -euo pipefail
export LC_ALL=C
[[ $# = 6 ]] || { echo 'usage: verify-cri-image.sh RESOLUTION_DIR CONTAINER_JSON IMAGE_JSON CONTAINER_ID POD_UID NEW_OUTPUT' >&2; exit 2; }
proof=$(realpath -- "$1")
runtime=$(realpath -- "$2")
image=$(realpath -- "$3")
id=$4
uid=$5
output=$(realpath -m -- "$6")
[[ "$id" =~ ^[0-9a-f]{64}$ && "$uid" =~ ^[0-9a-f-]{36}$ && ! -e "$output" && ! -L "$output" ]] || exit 2
for file in "$proof/result.json" "$runtime" "$image"; do
    [[ -f "$file" && ! -L "$file" && $(stat --format=%s -- "$file") -le 4194304 ]] || exit 2
done
mkdir -- "$output"
# Kubelet passes the resolved image ID in ContainerConfig.Image.Image. CRI
# preserves that creation configuration; containerd 2.3 also exposes imageId.
# imageRef is deliberately only recorded: containerd can replace it with the
# first repository digest associated with that config, not the original ref.
# See docs/oci-identity.md for pinned kubelet/containerd source references.
jq -ne --arg id "$id" --arg uid "$uid" \
    --slurpfile proof "$proof/result.json" --slurpfile runtime "$runtime" --slurpfile image "$image" '
    def digest: type == "string" and test("^sha256:[0-9a-f]{64}$");
    select(($proof | length) == 1 and ($runtime | length) == 1 and ($image | length) == 1) |
    $proof[0] as $p | $runtime[0] as $c | $image[0] as $i |
    select($p.schema_version == 1 and $p.passed == true and $p.os == "linux" and $p.architecture == "amd64" and
      ($p.root_digest | digest) and ($p.platform_digest | digest) and ($p.config_digest | digest)) |
    select($c.status.id == $id and $c.status.labels["io.kubernetes.pod.uid"] == $uid and
      $c.info.config.image.image == $p.config_digest and
      (($c.status | has("imageId") | not) or $c.status.imageId == "" or $c.status.imageId == $p.config_digest) and
      $i.status.id == $p.config_digest and $i.info.imageSpec.os == "linux" and $i.info.imageSpec.architecture == "amd64") |
    {schema_version:1,passed:true,container_id:$id,pod_uid:$uid,
      root_digest:$p.root_digest,platform_digest:$p.platform_digest,config_digest:$p.config_digest,
      creation_image:$c.info.config.image.image,runtime_image_id:($c.status.imageId // null),
      runtime_image_id_available:($c.status.imageId != null and $c.status.imageId != ""),
      observed_image_ref:($c.status.imageRef // null),
      binding:"Verified OCI config digest equals immutable CRI creation image ID and installed image ID"}
    ' > "$output/result.json"
sha256sum "$proof/result.json" "$runtime" "$image" > "$output/inputs.sha256"
