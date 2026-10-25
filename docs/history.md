# Implementation history

The current review stack consolidates the interrupted implementation. The complete reconciled source, original evidence and former review chronology remain at immutable commit [`1f9d4d7`](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e). Its archive branch is `archive/implementation-before-consolidation-20261019`; the original PR branches are retained. This archive commit preserves records; it does not restamp their producing revisions or establish a new release result.

Historical packets were removed from the delivery tree only after checking their Git blob IDs and executable modes, exporting the exact inventory, and actually restoring all 1258 files (5,071,752 bytes) with identical contents. Git already deduplicates equal objects; this reduces delivery and review clutter, not a claimed Git storage saving.

| Historical packet | Files | Git tree |
|---|---:|---|
| [docs/evidence/compose-canonical-diagnostic-20261018](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/compose-canonical-diagnostic-20261018) | 76 | `255c6a5d21b45c0f51103d811ea7982a8cb7c62e` |
| [docs/evidence/gateway-sentry-c405064](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/gateway-sentry-c405064) | 125 | `21dffd911b22100900b07685f6ed9078b3a1ea6a` |
| [docs/evidence/inferno-workspace-isolation-2026-10-19](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/inferno-workspace-isolation-2026-10-19) | 12 | `e84803ccc4f3004a753ed0bfb972a74f4cc5d81e` |
| [docs/evidence/kubernetes-historical-2026-10-19](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/kubernetes-historical-2026-10-19) | 28 | `58593e46f5488d3e5ea51fb473eab2091c7323e9` |
| [docs/evidence/native-service-stack-20261018](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/native-service-stack-20261018) | 9 | `9c3e4b42d28d16d85f6d1904fc651500860f2977` |
| [docs/evidence/profile-event-filter-20261018](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/profile-event-filter-20261018) | 9 | `ae0ee07c275e728a17f5407c2d212e626960f9b6` |
| [docs/evidence/recovery-20261018](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/recovery-20261018) | 13 | `31c136ba0e540681883e1f2ecf60772ae2079d4f` |
| [evaluation/build-cost/evidence/cache-pressure-2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/cache-pressure-2026-10-18) | 6 | `ff7f8a6110290b6851c8585986798381cd8e090f` |
| [evaluation/build-cost/evidence/chef](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/chef) | 6 | `fa0a4152ea131ae3289c44830ee11437e3818a64` |
| [evaluation/build-cost/evidence/generated-context-2026-10-19](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/generated-context-2026-10-19) | 10 | `9d21f2b416c9acde058dfd71a3943e3f66b9647f` |
| [evaluation/build-cost/evidence/independent-real-cargo](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/independent-real-cargo) | 14 | `4cab1a6b98ba0f007a8f355bbd75d08ba9a019c7` |
| [evaluation/build-cost/evidence/matched-4bae](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/matched-4bae) | 10 | `3d66aa5006e286964d6e3fc9867f289298668739` |
| [evaluation/build-cost/evidence/mimalloc-build-clock-2026-10-19](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/mimalloc-build-clock-2026-10-19) | 17 | `2193ff6853a18e3dae6b3981e5b21c76f79c4bc2` |
| [evaluation/build-cost/evidence/sdk-tooling](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/sdk-tooling) | 33 | `10e966419e741c0291dd2e06c16720cca94c2697` |
| [evaluation/build-cost/evidence/selector-real](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/selector-real) | 6 | `dff9288f04d7f3e675b755d22bc4d896412e4e77` |
| [tests/evidence/2026-10-18-native-profile-baseline](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/tests/evidence/2026-10-18-native-profile-baseline) | 722 | `8f714c8f116cda5aad5a86be9728f0ba17aa1130` |
| [tests/evidence/2026-10-18-private-profile-control](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/tests/evidence/2026-10-18-private-profile-control) | 162 | `f7b2011d1ce7a52e90b76045276323bb0c614a0a` |

Additional archived records (all verified against the same immutable commit):

| Path | Archived files | Removed delivery files |
|---|---:|---:|
| [evaluation/parity-replay/evidence/2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/evidence/2026-10-18) | 111 | 111 |
| [evaluation/parity-replay/test_evidence.py](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/test_evidence.py) | 1 | 1 |
| [evaluation/parity-replay/evidence/http-server-2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/evidence/http-server-2026-10-18) | 9 | 9 |
| [evaluation/telemetry-http/evidence/2026-10-19-correctness](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/telemetry-http/evidence/2026-10-19-correctness) | 318 | 318 |
| [runtime/validation/core-criterion](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/core-criterion) | 25 | 25 |
| [runtime/validation/core.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/core.json) | 1 | 1 |
| [runtime/validation/models](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/models) | 47 | 47 |
| [runtime/validation/perf-tool-probe/2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/perf-tool-probe/2026-10-18) | 69 | 69 |
| [runtime/validation/math-compaction/2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/math-compaction/2026-10-18) | 162 | 162 |
| [runtime/crates/indra-engine/tests/evidence/2026-10-18-parity](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/crates/indra-engine/tests/evidence/2026-10-18-parity) | 14 | 9 |
| [evaluation/build-cost/evidence/chef-layout-2026-10-18](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/evidence/chef-layout-2026-10-18) | 20 | 20 |
| [scripts/chef-layout.py](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/scripts/chef-layout.py) | 1 | 1 |
| [evaluation/build-cost/test_chef_layout.py](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/test_chef_layout.py) | 1 | 1 |
| [evaluation/build-cost/CHEF.md](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/build-cost/CHEF.md) | 1 | 0 |

The second restoration checked 780 files (3,907,421 bytes); 774 redundant paths left the delivery tree. Five original parity protocol/harness inputs remain active. `CHEF.md` remains as a pointer to the current cook procedure; its original bytes are archived. The parity self-test is restored with its frozen adapter and inputs as shown in the [replay guide](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/README.md).

The fresh parity replay's external raw archive remains separately identified as `parity-replay-issue13-20261018-evidence.tar.zst`, 155,079,229 bytes, SHA-256 `d7c4bd27f752d7e66af5324787e414dc9911d203fb66146684f707232359369a`. Its 2,108 raw files total 577,648,099 bytes. These external bytes are not embedded in the committed receipts.

Restore the entire telemetry packet before running `scripts/replay-telemetry-http.sh STATIC_VERIFIER RESTORED_PACKET NEW_OUTPUT`. Its original outer manifest, raw archive and pinned verifier identity are required together. This is offline correctness replay, not an overhead measurement.

Each tree ID binds the complete path, mode and content inventory. The original packet includes its source identities, failures, scope, raw-file receipts and original SVG/PNG assets. External models, ELFs, perf recordings and tensor arrays named by those receipts remain external; the Git archive does not claim to contain them.

To replay an archived packet, restore it into a new directory and pass that directory to its original replay command. For example:

```sh
archive=1f9d4d7b1185331189daec230fa7a34343258f4e
history=$(mktemp -d)
git cat-file -e "$archive^{commit}"
git archive "$archive" tests/evidence/2026-10-18-native-profile-baseline | tar -x -C "$history"
packet="$history/tests/evidence/2026-10-18-native-profile-baseline"
(cd "$packet" && sha256sum --check --strict SHA256SUMS)
# Use the exact frozen source checkout and full producing commit required by the packet:
bash scripts/replay-compact-native-profile.sh "$packet" "$frozen_source" "$producing_commit" "$new_output"
```

A shallow checkout must fetch the archive branch before using historical Git objects. Never overwrite current fixtures or regenerate old results to make replay pass.

The completed fixed-source trace diagnostic also remains in the archive: its [workflow](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/.github/workflows/trace-diagnostic.yml), controller, tests and original fixture retain the exact source-41 admission. Routine release and Tika workflows use the shared production trace-helper path.

The [former development guide](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/development.md) and [implementation roadmap](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/implementation-plan.md) retain the original PR sequence and reasoning. Current acceptance belongs in the [results ledger](results.md), including failed and unexecuted gates.

## Baseline-era records removed by the cleanup review

The Python-backend baseline left measured packets, a source-and-evidence report, a retained results snapshot, two research notes and the harness that produced them. They describe archived revisions and are not evidence about the native runtime; the cleanup review removed them from the delivery tree after checking that every byte is reachable at the immutable archive commit (or, for the harness, the baseline commit [`d6b37bc`](https://github.com/somehowchris/tika-indra-ocr-client/tree/d6b37bc)). `SOURCE_MANIFEST.json`, a 17 October snapshot of tracked-file hashes with no generator and no consumer, is superseded by the Git tree and blob IDs recorded here and by each packet's own `SHA256SUMS`.

| Removed path | Files | Git ID at `1f9d4d7` |
|---|---:|---|
| [validation/backend-real](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/backend-real) | 2 | `a8fae809681742c64170e253d93d466e281ab56e` |
| [validation/backend-rootfs.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/backend-rootfs.json) | 1 | `d1f3e7118c568e3469446c27616ad154ffc5e4b9` |
| [validation/bridge-profile-samples.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/bridge-profile-samples.json) | 1 | `0ff8db6f0cff22f9973761ba454cdb92648d39cc` |
| [validation/bridge-profile-summary.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/bridge-profile-summary.json) | 1 | `6afe6c1f9c4cb80dbbaa9113b3fbf8b73b6cfb9a` |
| [validation/bridge-protocol.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/bridge-protocol.json) | 1 | `f3cc21f5a07521a85c0b029f817c6d6ff5e61f0a` |
| [validation/ci](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/ci) | 95 | `ed1603ea23a773ab3754ebbb178ea7b93b5795eb` |
| [validation/final-review.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/final-review.json) | 1 | `9e15293015d4627a428feb3baf23d12e033146fb` |
| [validation/gnu-bridge-health.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/gnu-bridge-health.json) | 1 | `7ab0d44f14865cc04492bbb4fe05db07b093c258` |
| [validation/kubernetes-image-identity.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/kubernetes-image-identity.json) | 1 | `474338479a632cdabb26ad7d8f9ad879b6b6be8b` |
| [validation/relink-materials.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/relink-materials.json) | 1 | `c3228217aecd10fff553ad7743a8d73fab681cc2` |
| [validation/rootfs](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/rootfs) | 20 | `3161537fd1a96eb4035395f4cea3229fe5275746` |
| [validation/rust-checks.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/rust-checks.json) | 1 | `13709ebd9eddbba1afc578e241ad4eb0f04019b7` |
| [validation/tika-contract.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/validation/tika-contract.json) | 1 | `4fc7132f009d6c5add418d4192ee1dc9d0a9f1f5` |
| [IMPLEMENTATION_REPORT.md](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/IMPLEMENTATION_REPORT.md) (the two-line pointer banner that this review stack added on 20 October was never part of the archived file) | 1 | `9517426f157c997e3c9467c7d2d2ed6fcbb9d67d` |
| [docs/legacy-results.md](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/legacy-results.md) | 1 | `84c1a4feb03cf1ebfbbfdf49e8da38be06d5ff1f` |
| [research/benchmark-methodology.md](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/research/benchmark-methodology.md) | 1 | `5e2e132423e0d9ca374c1a253f4edd3e0d6013a2` |
| [research/benchmark-results.md](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/research/benchmark-results.md) | 1 | `f415bcfe475a902d69e89d03932beadabced680c` |
| [SOURCE_MANIFEST.json](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/SOURCE_MANIFEST.json) | 1 | `1c23b0d49c7db4dc4a1337c5dfc63f4836f82a08` |
| [benchmarks (harness: __init__, ci_docker, docker_run.py/.sh, generate_fixtures, make_local_plan, metrics, procstats, recover_cohorts, report, run, stack, requirements-fixtures.txt, public-regression.lock.json)](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/benchmarks) (`benchmarks/README.md` was rewritten in place) | 14 | — |
| [tests/test_benchmark_{ci,docker,image_archive,metrics,runner}.py](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/tests) | 5 | — |
| [scripts/{verify_kubernetes,install_syft,fetch_public_regression,measure_bridge,smoke_stack}.py](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/scripts) | 5 | — |
| [compose.benchmark.yaml](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/compose.benchmark.yaml) | 1 | `673ddc4f218b020c8267d5aaacdeda90aaedf7e1` |
| [benchmarks/results](https://github.com/somehowchris/tika-indra-ocr-client/tree/1f9d4d7b1185331189daec230fa7a34343258f4e/benchmarks/results) (the baseline packets, their receipts and the frozen backend and harness copies; `synthetic-fixtures/` and `bridge-streaming-2026-10-18/` stay, and the README's link edit made in this review stack is not archived) | 1417 | `508f9a76c230a74172f852ee78be00a000297ac1` |
