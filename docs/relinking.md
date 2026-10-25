# Static GNU bridge: source and relinking materials

The bridge links glibc statically. Each distributed bridge image includes `/relink` with the exact glibc source archives and Debian patches, bridge source and build inputs, license notices, and checksums. The Tika image copies this directory to `/relink/bridge`, alongside the gateway's separate bundle at `/relink/gateway`; the Tesseract comparison image inherits both. The directory contains no executable; its manifest records the associated executable's hash when collected during the image build.

GNU LGPL 2.1 section 6 requires notices and a route for recipients to modify the library and relink the executable. Section 6(a) allows the application in object code **and/or source code** when those materials support relinking. The supplied application source has no runtime Cargo dependencies. Criterion and the allocation counter are development dependencies; Cargo still resolves them when building the executable, so the commands below fetch the exact locked packages before the offline build. Its source, exact toolchain selection, build recipe and the actual library source are included together. See the [license text, section 6](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html), also carried locally as `/relink/licenses/LGPL-2.1`. This document describes the technical materials supplied; it does not replace the component licenses.

## What the build collects

The Debian builder enables the signed snapshot's `deb-src` repository and downloads the source version reported by the installed `libc6-dev`. It then runs the same Bash collector as the native service and gateway images:

```sh
bash scripts/collect-runtime-materials.sh bridge /relink /indra-tesseract
```

The collector requires a matching glibc `.dsc`, upstream source tarball and Debian packaging/patch archive. It verifies the size and SHA-256 of every source file named by the `.dsc`; missing files, mismatched versions or incorrect hashes fail the build. APT supplies the repository-signature verification. The collector does not independently verify OpenPGP signatures.

| Bundle path | Contents |
|---|---|
| `glibc-source/` | Exact `.dsc`, original sources and Debian modifications, listed in `glibc-archives.txt` |
| `rust-source.tar.gz` | Rust source, Cargo manifests/lock, toolchain selection, vendored crates with an offline `.cargo/config.toml`, Dockerfile, collector, component SBOM writer and these instructions |
| `licenses/` | glibc, libgcc and libstdc++ copyright notices, GNU license texts, Rust toolchain notices and the Indra code/model licences |
| `build-identity.txt` | Installed glibc source version, the executable's SHA-256, `rustc -Vv`, `gcc --version` and `getconf GNU_LIBC_VERSION` |
| `static-components.spdx.json` | SPDX 2.3 packages, with purls, for the statically linked glibc and libgcc archives at the builder's Debian package versions; release signing merges them into the signed SBOMs of the bridge, Tika and Tesseract images |
| `SHA256SUMS` | Checksums for all delivered files |

Preserve `/relink` when repackaging an image. When distributing a bare executable, provide these materials alongside that exact executable, with a prominent link from the release page. A source-only project checkout does not contain the downloaded glibc archives; the image build acquires and includes them. Do not present a moving upstream download URL as a substitute for the actual corresponding source supplied with a release.

## Extract and verify

For a running local Tika service:

```sh
docker cp "$(docker compose ps -q tika)":/relink/bridge ./relink
cd relink
sha256sum --check SHA256SUMS
```

`build-identity.txt` identifies the exact library source version (`glibc_source_version=`). Keep it with the files. The original license and copyright notices remain applicable when a recipient makes changes.

## Relink with an ABI-compatible modified glibc

Use the Debian build stage from `docker/bridge.Dockerfile` as the toolchain environment. Keep that builder at a maximum of two logical CPUs through the container runtime. The following commands describe work **inside that writable builder**, with the collected bundle available at `/work/relink`. They do not write into the read-only runtime image.

```sh
apt-get update
apt-get install --yes --no-install-recommends dpkg-dev
mkdir -p /work/glibc-build /work/output
cp -a /work/relink/glibc-source/. /work/glibc-build/
mkdir -p /work/bridge && tar -xzf /work/relink/rust-source.tar.gz -C /work/bridge
cd /work/glibc-build
dpkg-source --extract ./*.dsc glibc
```

Install glibc's build dependencies using the same signed source repository and the source version in the manifest. Make the intended library change in the extracted source, and record it in the Debian patches/changelog, including a distinguishable package version. Build both the modified source package and matching binary packages. The build may need more resources and time than the small bridge itself; no full modified-glibc build is included in the OCR performance measurements.

```sh
RELINK_GLIBC_VERSION="$(sed -n 's/^glibc_source_version=//p' /work/relink/build-identity.txt)"
apt-get build-dep --yes --only-source "glibc=${RELINK_GLIBC_VERSION}"
cd /work/glibc-build/glibc
# Apply and document the intended ABI-compatible library changes here.
DEB_BUILD_OPTIONS=parallel=2 dpkg-buildpackage -F -us -uc
```

Install the resulting, mutually matching amd64 libc runtime/development packages into the builder, replacing its `libc6-dev` and static archives. The package globs below are for the supported `linux/amd64` target. Review the selected filenames before installation when the build directory contains more than one version.

```sh
apt-get install --yes --reinstall \
  /work/glibc-build/libc6_*_amd64.deb \
  /work/glibc-build/libc-bin_*_amd64.deb \
  /work/glibc-build/libc-dev-bin_*_amd64.deb \
  /work/glibc-build/libc6-dev_*_amd64.deb

cd /work/bridge
cargo fetch --locked
cargo clean --target x86_64-unknown-linux-gnu
RUSTFLAGS='-C target-feature=+crt-static' \
  cargo build --locked --offline --jobs 2 --profile release \
  --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/indra-tesseract /work/output/
readelf --program-headers /work/output/indra-tesseract
readelf --dynamic /work/output/indra-tesseract
```

The rebuilt bridge should retain no ELF interpreter or shared-library `NEEDED` entries. Run the adapter protocol checks and the actual Tika/backend integration against the modified binary before using it. A changed library creates a new binary with new provenance; previous timing results do not automatically cover it.

For redistribution, update the build recipe to reproduce the modified library, keep its complete source and changes, and collect a fresh bundle using the modified `.dsc` and the newly installed `libc6-dev`. The collector deliberately rejects a source version that does not match the installed development package.

The local verification in this project validates real source acquisition, version matching, archive hashes, and notice/material collection. It does not claim a full rebuild with an intentionally modified glibc was performed.
