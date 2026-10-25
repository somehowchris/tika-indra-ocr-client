An experimental signed SDK seed can reuse a previously published native closure
while repairing the separate application/Chef proof. It is not a reviewed
`native-inputs.lock.json`, and it supplies no prior fresh-proof or cost claim.
No seed values or enabled selection are included here.

The optional release dispatch input `proof_sdk_seed` names a committed,
repository-relative JSON file following
[`native-sdk-seed.schema.json`](../../docker/native-sdk-seed.schema.json).
Its metadata directory, relative to that file, contains the exact producer's
`index.json`, `index.sigstore.json`, `sdk.spdx.json` and
`sdk-materialized/materialization.json`. All five files must be canonical regular
files whose actual bytes equal their `HEAD` Git blobs. Files and aggregate
metadata are bounded. Producer and current application identities remain distinct.

Selection is allowed only in this repository's existing `release.yml` publish job
on a `v*` tag with `workflow_dispatch`, `native_input_mode: sdk` and
`run_sdk_proof: true`. Actual dispatch inputs, workflow identity and checked-out
SHA are checked before registry setup. PRs, tag pushes, routine builds, foreign
repositories, disabled proof and other modes fail. A supplied seed never becomes
a source-build fallback.

The prior release attempt must be complete, and its exact publish job must have
succeeded. If the whole attempt failed, its complete API job list must show that
the separate fresh SDK/Chef proof was its sole failed job. That failure is retained
in the unpromoted selection receipt. Index and image signatures still require the
same exact release v-tag, source SHA, repository and GitHub OIDC policy. Exact SPDX
and input predicates, raw producer OCI receipt, every downloaded OCI blob and the
current native recipe use the existing admission path. Actual compiler and closure
verification run inside the generated consumer before application compilation.

All current production native gates and exported baselines use that consumer.
The optional SDK producer renders the normal producer, then the existing consumer
renderer replaces native compilation with the verified seed. Packaging writes a
new current producer identity; the old SDK remains explicit parent provenance,
bound into the new signed index and transported packet. Chef cooks current source
with the shared epoch-zero application environment and a new compatible key; it
does not restore old cooked artifacts.

The six-image runtime and signing gates, separate fresh executable-role equivalence,
materials, native/lifecycle gates, invalidation negatives and paired total-cost
proof remain mandatory. Routine pinning still requires the unchanged complete
proof and reviewed net-cost decision described in [ADOPTION.md](ADOPTION.md).
Synthetic tests exercise orchestration, committed Git bytes and real OCI corruption
rejection. They do not prove an actual signed seed or demonstrate a speedup.
