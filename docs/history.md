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

The second restoration checked 780 files (3,907,421 bytes); 774 redundant paths left the delivery tree. Five original parity protocol/harness inputs remain active. `CHEF.md` remains as a pointer to the current cook procedure; its original bytes are archived. The parity self-test is restored with its frozen adapter and inputs as shown in the [replay guide](../evaluation/parity-replay/README.md).

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
