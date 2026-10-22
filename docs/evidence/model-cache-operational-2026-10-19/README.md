# Model-cache operational negatives

Four real CLI cases passed using the existing static source-`5ae8b82` server,
controller `68bf29e` (exact producing source included),
and an existing GNU helper image. Cache, model-lock and command source have zero
Git diff against source `60c849a`; this scoped comparison does not establish
whole-application equality or current-source release acceptance.

| Case | Actual observation |
|---|---|
| Unprivileged cache write | UID/GID 65532 cannot open the writer lock in the host-owned mode-0555 directory: EACCES, exit 1. The marker and directory metadata stay unchanged. |
| Cold cache without network | The empty cache cannot resolve the exact immutable model URL; exit 1. No artifact or partial is published; only the empty writer-lock file remains. Both request deadlines are configured to one second. |
| Unsupported revision flag | `--model-revision` is an unknown option, exit 2. |
| Unsupported revision config | `models.revision` is an unknown typed field, exit 1; its value is absent from the diagnostic. |

Each fresh container has an actual numeric one-CPU quota, CPU 12 affinity,
512 MiB memory, zero swap, 64 tasks, private cgroup/PID namespaces, no network,
no capabilities and a read-only root. The live host UID/GID, process affinity,
Docker owner label, quota/memory ancestors, executed command and final status are
retained. The wrapper and CLI share that cgroup. All four containers exit with
their expected status and are removed; the controller and cleanup exit zero.
This is functional correctness evidence, with no inference, download or speed claim.

The native executable is the exact 59,959,728-byte `bin/indra-server` member of
[failed release run 39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394),
artifact `12172057374`, SHA-256
`1d5d78413ffcf1782f0488dcac4fae7dce807651e39cf72a750757828d81cbc2`.
Its real Cargo selection and ELF records are included. The original ZIP transport
receipt is retained; this addendum did not rehash or change that sealed archive.
The GNU image identifies only the diagnostic helper environment; it is not a
signed release or SDK-adoption claim. Repeat these cases using the actual accepted
source-`60c849a` ELF before claiming current-source acceptance.

`raw.tar.gz` contains all 96 attempt files, the original small producer receipts,
and the exact producing controller. The 60-MB executable is omitted and remains
available in the named original artifact. To inspect the complete small raw packet:

```sh
sha256sum -c SHA256SUMS
mkdir /absolute/new-cache-proof
cd /absolute/new-cache-proof
tar -xzf /absolute/checkout/docs/evidence/model-cache-operational-2026-10-19/raw.tar.gz
sha256sum -c RAW_SHA256SUMS
```

Run the controller from a clean checkout with an already admitted GNU helper
image containing Bash/coreutils, the extracted static CLI and its actual producer
hash. It does not pull images or download models:

```sh
scripts/verify-model-cache-negative.sh \
  sha256:GNU_HELPER_IMAGE_CONFIG_DIGEST /absolute/indra-server \
  ACTUAL_ELF_SHA256 ACTUAL_PRODUCER_COMMIT /absolute/new-output 12
```

Revision selection is deliberately narrow. The native CLI bakes the immutable
repository/revision and verifies every selected artifact's size/hash. Directory
names are arbitrary; content-identical files are valid regardless of provenance
or folder label. The tests above do not acquire a different upstream snapshot and
do not relabel an ordinary content-mismatch fixture as one. Existing manifest,
size/hash, interrupted-write and atomic-preservation tests remain separate evidence.
