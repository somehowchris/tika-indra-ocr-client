# Native GNU runtime: source, notices, and relinking

The native OCR and gateway images contain static GNU executables. Each carries
its own `/relink` directory with application and vendored Cargo source, exact
glibc source archives and Debian patches, notices, build identities, and a
relative checksum inventory. The native OCR bundle also contains the exact
ONNX Runtime/OpenCV archive closure and installed OpenCV headers. Each bundle's
`static-components.spdx.json` names the statically linked C/C++ components as
SPDX packages with purls; release signing merges them into the signed SBOM of
each image that ships the executable (see
[container deployment](container-deployment.md#verify-signed-images-before-deploying)).
The bridge's separate materials remain documented in [relinking.md](relinking.md).

This supplies a practical path to modify an ABI-compatible glibc and relink the
application. GNU LGPL 2.1 section 6(a) permits application source and/or object
code for this purpose; see the [license text](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html),
also included locally. Component licenses and exceptions remain applicable.
The model license is separate; source availability does not change its terms.

## Extract and verify

Create an unstarted container from the exact image digest, copy `/relink`, then
remove that temporary container:

```sh
docker create --name indra-materials IMAGE@sha256:DIGEST
docker cp indra-materials:/relink ./relink
docker rm indra-materials
cd relink
sha256sum --check SHA256SUMS
```

The bundle identifies the executable by SHA-256. The builder obtains glibc
through signed, pinned Debian snapshot metadata. The collector independently
checks its installed source version and every `.dsc` archive hash and size; it
does not independently verify the `.dsc` OpenPGP signature.

## Rebuild and relink

Use the exact GNU builder from `docker/native.Dockerfile` for OCR, or
`docker/gateway.Dockerfile` for the gateway. Work in a writable builder with
at most two build jobs. Extract `rust-source.tar.gz` into `/app`. For native OCR,
extract `native-link-inputs.tar.gz` at `/`; the preserved `/native` paths match
the explicit build environment. No native shared-library search is involved.

Use the included glibc `.dsc` and archives to build the intended modified
library and its matching amd64 development/runtime packages. The corresponding
Debian source instructions are in [the bridge relinking guide](relinking.md#relink-with-an-abi-compatible-modified-glibc).
Install those packages in the builder before the final link. Release images
bake their source commit into `service.version` and the Sentry release; export
it as `INDRA_SOURCE_REVISION`, taken from the image's
`org.opencontainers.image.revision` label, or the rebuilt executable reports
the crate version alone. Then run:

```sh
cd /app
export INDRA_SOURCE_REVISION=COMMIT
# Native OCR only:
. /native/cargo-env.sh
# For the gateway instead:
# export RUSTFLAGS='-C target-feature=+crt-static'
cargo build --locked --offline --jobs 2 --release \
  --target x86_64-unknown-linux-gnu --manifest-path runtime/Cargo.toml \
  --package indra-server
# Substitute --package indra-gateway for the gateway executable.
readelf -W -l runtime/target/x86_64-unknown-linux-gnu/release/indra-server
readelf -W -d runtime/target/x86_64-unknown-linux-gnu/release/indra-server
```

Require no ELF interpreter and no `NEEDED` entries, then rerun the real protocol,
native inference and Tika lifecycle gates. A modified library produces a new
binary with new provenance; previous accuracy and performance results do not
cover it automatically. Preserve complete modified glibc sources and collect
a new bundle before redistribution.

The collected materials and their hashes are build gates. This document does
not claim that an intentionally modified glibc has been built or benchmarked.
