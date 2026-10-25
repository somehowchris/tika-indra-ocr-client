# Bind images at the correct OCI level

The deployment and release gates distinguish three content identities: the root
index, its selected platform manifest and the manifest's image configuration.
They are different SHA-256 values. The gates verify their relationship before
comparing an actual container with an expected image.

## Why the original gate failed

On Docker 29.9.1 with its containerd image store, an unqualified image inspection
returns the root descriptor as both `.Id` and `.Descriptor`. Adding
`--platform linux/amd64` returns the selected manifest instead. A Docker
container records the root in `.Image` and its actual selected manifest in
`.ImageManifestDescriptor`. The implementation is visible in the producing
daemon's [image inspection source](https://github.com/moby/moby/blob/b26fb77/daemon/containerd/image_inspect.go).

In [the retained failed run](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/recovery-20261018/receipt.json), Tika's root
index starts with `6dcf78ff` and its selected manifest starts with `c518b564`.
Both services became healthy. Comparing those two different levels rejected the
stack before the real accepted-work shutdown test.

## Docker and archive verification

[`verify-daemon-image.sh`](../scripts/verify-daemon-image.sh) resolves the image
reference once, then inspects and exports its immutable root digest. Export
omits `--platform`: the daemon's [exporter](https://github.com/moby/moby/blob/b26fb77/daemon/containerd/image_exporter.go)
otherwise substitutes the selected manifest for the original index.

The verifier accepts a single manifest or an index with exactly one unambiguous
linux/amd64 manifest. It reads bounded, uniquely named, regular metadata members
without extracting filesystem layers. It checks the raw SHA-256 and byte length
at each descriptor edge, the selected platform, raw configuration and rootfs
identities, and both daemon inspections. Before accepting a Docker container,
it verifies its root and actual manifest descriptor against that graph again.
An unrelated tag or `RepoDigests` entry cannot satisfy the gate.

Only the containerd image store is supported by this gate. A classic image store
whose `.Id` is a configuration digest and lacks the original OCI descriptor is
rejected with an explicit diagnostic. The pinned Actions daemon enables the
required store. This is a gate dependency, not a requirement for every user who
runs a published OCI image.

The helper authenticates OCI metadata relationships. Docker/containerd performs
the layer-payload verification during image import/pull. Registry provenance,
the signed source revision, production/test executable equality and static ELF
checks remain separate release checks.

## Kubernetes uses its actual configuration identity

The Kubernetes gate saves the tagged image without selecting a platform, so kind
can import its name. It verifies that second archive against the earlier root
and selected-platform inspections. A changed tag during export fails before
import. The cluster is newly created and owned by this gate.

Kubelet [passes the resolved image ID](https://github.com/kubernetes/kubernetes/blob/v1.36.4/pkg/kubelet/kuberuntime/kuberuntime_container.go)
in `ContainerConfig.Image.Image` when creating the container. Containerd retains
that creation configuration in verbose CRI inspection. Its displayed
`status.imageRef` can be replaced by the first repository association for the
configuration, as shown in the [2.2.3 status implementation](https://github.com/containerd/containerd/blob/v2.2.3/internal/cri/server/container_status.go).
It is retained for diagnosis and does not establish acceptance.

[`verify-cri-image.sh`](../scripts/verify-cri-image.sh) compares the immutable
creation image ID and the installed CRI image ID with the configuration digest
already authenticated by the OCI graph. It binds the original container ID and
pod UID, and checks linux/amd64. When the runtime supplies `status.imageId`, as
in [containerd 2.3](https://github.com/containerd/containerd/blob/v2.3.0/internal/cri/server/container_status.go),
that field must agree too. An absent or empty string is recorded as unavailable:
[`crictl` emits default string values](https://github.com/kubernetes-sigs/cri-tools/blob/v1.36.0/cmd/crictl/util.go)
even when an older runtime does not supply the field. Null and other malformed
present values fail validation. This is explicitly a CRI configuration binding; it
does not pretend that the CRI display field identifies the selected manifest.

The offline fixtures exercise valid index/single-manifest and older/current CRI
records, together with changed identities, ambiguous platforms, malformed or
duplicate metadata and overwrite rejection. They precede expensive builds.
Actual Compose/Kubernetes lifecycle acceptance still requires a fresh hosted
run; fixture success alone cannot prove deployment shutdown.
