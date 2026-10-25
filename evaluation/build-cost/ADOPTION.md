The optional native-input action prepares a current-source build from an already
proved, reviewed producer. It does not publish an SDK, expand signing trust or
turn a completed cost schedule into a speedup claim. No production lock or input
values are included in this source change.

A future actual `docker/native-inputs.lock.json` follows
[`native-inputs.schema.json`](../../docker/native-inputs.schema.json). Its producer
identity includes the exact repository, trusted `v*` tag, source SHA, release run
and attempt, release workflow identity and GitHub OIDC issuer. These are distinct
from the current application checkout SHA. The lock binds the signed index and
bundle, immutable SDK/Chef references, original artifact identities, successful
fresh proof job and the hashes of the actual proof/cost summaries. Its net-cost
rationale records the reviewed decision, including producer and transport costs.
Choosing Chef requires evidence that its net cost is useful; completing three
pairs alone does not demonstrate that.

Keep these bounded public metadata files beside the actual lock in its relative
`metadata_directory`:

- `index.json`, `index.sigstore.json`, `sdk.spdx.json`, `chef.spdx.json`.
- `sdk-materialized/materialization.json`, `chef-oci-integrity.json`, and `chef-manifests/key.json` from the exact signed producer.
- `proof-result.json`, `cost-result.json`, and `cost-protocol.json` from its completed fresh proof/cost job, renamed from that job's corresponding result/protocol paths without changing bytes.

This data remains reviewable after the original Actions artifact expires. Routine
builds do not download executable baselines or the large proof archive. They
recheck the exact successful proof job/attempt through GitHub, verify the index
bundle and image signatures against the producer identity, compare exact SPDX
and input predicates and read every selected OCI blob. SDK materialization checks
the current native recipe against the pinned source closure. The actual builder
then checks its exact toolchain, native configuration, closure inventory and file
bytes before compiling current application code. Inputs failing trust, integrity,
compatibility or availability stop the build.

Routine engine, service, deployment and release workflows use the action. The
profiling workflow explicitly chooses SDK-only restoration: its stages inherit
`native-build`, retain their original jq installation and complete profile export,
and receive none of the Chef release target state. Original Cargo commands and
all current-source runtime/deployment gates remain in place. Generated recipes
stay under `docker/`, preserving the existing release provenance path contract;
their hashes and producer/current-source identities are retained separately.

The selection is explicit before any build input is restored:

- With no lock (and no release seed), every mode selects the original source Dockerfile; the source mode is recorded next to the requested selection.
- `native_input_mode: source` explicitly selects the original native source build. Use this for a changed native recipe, then create and prove a new SDK before pinning it.
- `native_input_mode: sdk` compiles current Rust dependencies and application code using the reviewed lock's verified SDK. Use this for legitimate Cargo locks, manifests, features, configuration or external shared-source edits; no native SDK republication is required.
- `native_input_mode: locked` uses the lock's reviewed `sdk` or `chef` choice. Before any Chef download/restore, its signed dependency key is compared with current workspace/tooling/native inputs. A mismatch asks for the explicit SDK-only choice and fails. It is never caught as a cache miss or a reason to ignore a signature/content error.

Dispatch inputs expose these choices; profiling accepts only `sdk` and `source`.
For PRs, a reviewed lock can explicitly choose `sdk`, or its removal can select
the original source path for a deliberate native recipe change. No error path
falls back automatically. GHA cache leaves remain imports only; this adoption
adds no cache exporter. The separate opt-in SDK producer uses the original source build for a new trusted
recipe. Without a seed it reuses the native layer that the release's source-built
gates leave on the builder, so an unseeded `run_sdk_proof` dispatch needs the
source path (`native_input_mode: source`, or no lock); otherwise the release stops
right after input selection, before any build. A narrowly gated [experimental signed SDK seed](SDK-SEED.md) can reuse an
unchanged verified native closure for a release-only proof experiment. It does not
relax routine adoption requirements. Actual pinning and enablement must
wait for successful fresh proof and a reviewed net-cost decision.
