# Immutable model cache

`indra-models` gives both the native engine and explicit setup command one
artifact identity. Serving is offline: a missing or corrupt model is a startup
error, never an implicit download in an OCR request.

## Contract

The [authoritative lock](../backend/models.lock.json) pins Hugging Face repository
`datalab-to/indra-ocr` at revision
`ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`. The generated Rust manifest records each
filename, byte length, SHA-256 and math-profile membership. A test compares every
entry against that lock, including the repository and revision.

| Selected profile | Artifacts | Stored bytes |
|---|---:|---:|
| Text, including CJK | 9 | 18,313,491 |
| Text and math | 14 | 38,187,650 |

The cache should be application-owned and revision-scoped, for example
`/models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`. Mount it read-only in serving
containers. At startup, verify the entire selected profile, load every worker's
sessions, and warm the workers before reporting readiness. Each engine load also
verifies its inputs; startup verification is outside timed steady-state OCR.

Verification rejects missing, truncated, oversized, altered and symlinked
artifacts, non-regular files, a symlink as the final cache directory, and additional
unlocked `.onnx` filenames. File descriptors use `O_NOFOLLOW | O_NONBLOCK`; metadata
is checked after opening so a FIFO cannot block the regular-file check. SHA-256
uses a reusable 64 KiB stack buffer. This is a Unix/GNU implementation.

## Acquisition and recovery

The default crate feature set has no HTTP client. Enable `fetch` only in the setup
executable. `fetch_with` accepts positive connect and total-request deadlines;
the library's `AcquisitionOptions::default` is 10 s and 120 s, while the service CLI
(`models.connect_timeout_seconds` / `models.request_timeout_seconds`) defaults to
10 s and 300 s so a cold fetch of the full model set fits one attempt. Calls are synchronous setup work before
Tokio starts, or work for a separately scheduled init container.

1. Create the cache directory and acquire a non-blocking OS lock on `.fetch.lock`.
   A competing cooperating fetch returns a typed `Busy` error; descriptor closure
   releases the lock, including after an error or process exit.
2. Verify existing artifacts. Reuse valid files; download only missing or corrupt
   files from the immutable revision over HTTPS. Redirects are bounded to five
   and remain HTTPS-only.
3. Stream into `<artifact>.part`, created exclusively with mode `0600`. Check the
   exact byte count and SHA-256 while writing; the input buffer stays 64 KiB.
4. Set mode `0444`, sync the file, rename it atomically to its final filename and
   sync the directory. A failure before publication preserves an existing good
   final artifact. The next cooperating fetch removes the known stale partial.

Publication is atomic **per artifact**. It is not a transaction across all 14
files. Complete-profile verification is the admission gate for serving. A reader
started during an incomplete acquisition fails startup and can be restarted;
prefer an init container or build step that finishes acquisition first.

The OS lock coordinates this implementation's writers on a local filesystem
with working advisory locks. Initialization has one writer; this is not a
distributed lock for a shared RWX cache. It does not prevent an
uncooperative writer changing files, nor protect writable ancestor directories
from adversarial replacement. SHA-256 does not close the interval between
verification and ONNX Runtime opening a path. Read-only serving mounts and cache
ownership provide that operational boundary. Disk power-loss behavior has not
been fault-injected.

## Development and verification

From the repository root:

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt,clippy
cargo +1.99.1 test --locked --manifest-path runtime/Cargo.toml -p indra-models --all-features
cargo +1.99.1 clippy --locked --manifest-path runtime/Cargo.toml -p indra-models --all-targets --all-features -- -D warnings
```

The default tests need no network. Two explicitly ignored integration tests make
real acquisition and offline verification reproducible:

```sh
# Select a NEW directory: the acquisition test refuses to overwrite one.
INDRA_TEST_FETCH_DIR="$PWD/model-cache-proof" \
  cargo +1.99.1 test --locked --manifest-path runtime/Cargo.toml \
  -p indra-models --all-features fetch_real_immutable_revision -- --ignored --nocapture

INDRA_TEST_MODEL_DIR="$PWD/model-cache-proof" \
  cargo +1.99.1 test --locked --manifest-path runtime/Cargo.toml \
  -p indra-models verify_real_locked_cache -- --ignored --nocapture
```

The service exposes `indra-server models fetch` and `indra-server models verify`.
Select the same `--model-dir` and `--math true|false` settings for acquisition,
verification and serving. Serving never acquires models. The fetch client honors
normal proxy settings; the native image supplies `SSL_CERT_FILE` for CA trust.

## Evidence and performance scope

The [validation record](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/models/validation.json) retains the
source and binary hashes, commands and raw logs. The original nine default unit tests passed,
including replacement failure, partial recovery, concurrent writers, manifest
drift, content corruption and symlink rejection. Independent review found no
blocking defect under the documented application-owned cache boundary.

The real Rust downloader acquired all 14 locked files (38,187,650 bytes) and then
reused all 14 with **zero downloads** on a second call. Offline verification passed
with one and two allowed CPUs. These are debug-build correctness checks: their
elapsed times are retained for provenance, **not a cache speed comparison**. The
two verification runs overlapped briefly, so their timing difference is not
interpretable. They use affinity, not CFS quotas.

Release Criterion runs measure **offline verification of a warm filesystem
cache**, with mimalloc, 100 samples, one second warmup and a three-second target.
Criterion extended collection to retain all 100 samples. The function is
single-threaded; allowing two CPUs does not make it parallel.

| Allowed CPUs | Profile | Before, mean ms [95% CI] | Reused path, mean ms [95% CI] |
|---:|---|---:|---:|
| 1 | Text | 23.72 [22.30, 25.39] | 30.36 [27.01, 35.09] |
| 1 | Math | 50.87 [48.11, 54.16] | 90.78 [82.79, 100.21] |
| 2 | Text | 22.64 [21.01, 25.00] | 30.92 [26.80, 36.24] |
| 2 | Math | 50.84 [46.61, 56.57] | 62.31 [57.42, 68.43] |

The later runs were slower. Fixed before/after order and other compilation on
different CPUs sharing this host confound attribution; these intervals quantify
within-run sampling uncertainty, not those confounders. Keep every sample and
outlier. **These runs do not establish a cache latency improvement.** Full engine
startup, cold storage, and the deployed CFS workload need separate measurements.

The allocation experiment supports a narrower decision: allocate the artifact
path once with sufficient capacity and change its filename between files.
`stats_alloc` observes System separately from Criterion's mimalloc timing. Across
100 complete passes, both CPU widths produced identical counts:

| Per complete verification | Before allocations / reallocations / requested bytes | Reused path |
|---|---:|---:|
| Text | 45 / 9 / 1,291 | 37 / 0 / 562 |
| Math | 50 / 14 / 1,731 | 37 / 0 / 562 |

All observed allocations were deallocated. Retain path reuse for this measured
reduction in churn; do not advertise it as faster startup. The fixed hash/write
buffer already avoids per-chunk allocation. Directory iteration, the one path
buffer and setup HTTP/TLS still allocate. This is no zero-allocation claim for
cache setup, OCR or dependencies. The raw evidence includes a reverse patch and
source hashes to reproduce the pre-change implementation.

## Operational negative acceptance

The [complete small raw packet](evidence/model-cache-operational-2026-10-19/README.md)
adds actual unprivileged EACCES and cold, network-disabled CLI failures. It also
checks rejection of unsupported model-revision CLI/configuration fields. The
static source-`5ae8b82` executable is source-qualified to the unchanged source-`60c849a`
cache/command closure; it is not current-source release acceptance. Repeat the
controller with the accepted current release executable and retain its own identity.
No OCR or model download is performed by these checks.

Revision identity comes from the baked immutable manifest and per-file content,
not the cache directory's name. There is no public revision override. Correct
locked bytes remain valid under an arbitrary directory name; different bytes must
satisfy the existing size/hash checks. No separately acquired foreign snapshot is
claimed by this addendum.

## Sources and licensing

- [Pinned model repository](https://huggingface.co/datalab-to/indra-ocr/tree/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8)
  and [model license](https://huggingface.co/datalab-to/indra-ocr/blob/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/LICENSE).
  The code's Apache-2.0 license does not replace the model's Modified OpenRAIL-M
  terms. Review the pinned license, including commercial restrictions, before
  distributing weights or deploying commercially.
- [Rust `File::try_lock`](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)
  documents lock contention, descriptor lifetime and platform behavior;
  [`sync_all`](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all)
  documents explicit error reporting for synchronization.
- [Kubernetes init containers](https://kubernetes.io/docs/concepts/workloads/pods/init-containers/)
  provide sequential setup before application containers. Acquisition belongs in
  this phase when models are not already included in the immutable image.

Source review and recorded checks: 2026-10-17 UTC.
