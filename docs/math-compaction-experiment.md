# Same-age math decoder experiment

This opt-in diagnostic removes finished rows from one math cohort. It is absent
from `EngineConfig`, the service, and default builds. The ordinary decoder is
unchanged. This study does not insert new BOS rows into an existing history:
Indra uses a scalar position shared by the cohort and a common self-cache time
axis ([pinned upstream decoder](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py)).

The [prospective protocol](../runtime/validation/math-compaction-protocol.json)
was committed before candidate inference. The existing fourteen-case comparison
and quality study keep their original settings and acceptance criteria.

Build the isolated diagnostic with the same pinned static native libraries and
Rust flags as the native image:

```sh
cargo build --locked --release --manifest-path runtime/Cargo.toml \
  --target x86_64-unknown-linux-gnu -p indra-engine \
  --features math-compaction-experiment --example math_compaction
cargo test --locked --release --manifest-path runtime/Cargo.toml \
  --target x86_64-unknown-linux-gnu -p indra-engine \
  --features math-compaction-experiment --test math_compaction_allocations
cargo bench --locked --manifest-path runtime/Cargo.toml -p indra-engine \
  --features math-compaction-experiment --bench math_compaction
```

First generate an ordinary production trace from the supplied synthetic fixture
using the same newly linked engine, threads, immutable models and flags. The
trace directory must be empty; do not enable tracing during measured runs:

```sh
ocr runtime/crates/indra-engine/tests/data/math.png --model-dir /models \
  --math --batch-size 64 --threads 1 --cv-threads 1 \
  --trace-directory /evidence/reference --output /evidence/ordinary.txt
math_compaction --model-dir /models --reference /evidence/reference \
  --threads 1 --arm compact --gate --resources /evidence/gate-resources
```

The gate rejects a trace containing more than one math cohort. It checks the
replayed ordinary encoder, mask, cross caches, logits and token/stop snapshots
against the trace from `Engine::recognize`. An independent ordinary stream stays
at six rows. Before and after every candidate step, the gate compares live rows
by their original cohort index, including both self caches, both cross caches,
mask, logits, tokens, probability history, stop reason/step, token history,
confidence acceptance and decoded UTF-8. It also checks each complete accepted row string directly against feature-only snapshots from the actual production decision path, including rejected/empty rows. Floats must be finite and satisfy the
unchanged `atol=rtol=1e-5`; discrete decisions and text are exact. Failures print
the scalar step, live original-row map, stage, element and values to stderr.

The fixture must actually execute `6 → 4 → 3 → 2 → 1` live rows, ending the
original rows at steps `[8, 32, 8, 27, 60, 23]`. This is an implementation test,
not evidence about mathematical OCR quality on real documents.

Each gather uses fixed arrays and `copy_within` on already-owned CPU outputs.
The batch stride is the current logical batch, including after repeated
compaction; physical ORT output shapes can still describe the larger allocation.
Borrowed prefix `TensorRef` inputs expose the compact shape during the next
synchronous call. No extra maximum-sized cache arena or unsafe code is needed.
In numerical gates, every moved block is checked against its still-intact source bits immediately. Timed runs use the same const-generic copy implementation with verification compiled out, and report zero verified elements.
Independent sentinel tests construct expected values from immutable original
coordinates, including multiple layers, heads, cache time, repeated gathers,
signed zero, boundaries and invalid selections.

The allocation executable measures only prepared gather/control operations with
`stats_alloc` and the System allocator. Criterion measures the same copy core with verification disabled on reused arenas. Neither establishes whole-decoder zero
allocation: native outputs and ORT input metadata allocate. The JSON report counts
constructed tensor views and actual copied bytes separately for cross K/V, self
K/V, mask and tokens. It also reports native decoder row-steps. A reduction in row
steps is not a measured latency improvement.

For timing, pass `--warmup`, omit `--gate`, and start a fresh container/cgroup for
each arm and repetition. Alternate ordinary/compact order for ten paired
repetitions at each of one and two CPUs. Keep model files read-only and warm;
apply the total quota to the entire process, disable network and swap, and set
`--memory 3g --memory-swap 3g`. Bind suitable distinct physical cores on the same
cache domain and retain effective quotas/affinity and parent limits. Use the same
pinned static diagnostic in a scratch final image or mount it read-only into the
native distroless/scratch image; use UID 65532. See the protocol for evidence and
rejection requirements.

```sh
math_compaction --model-dir /models --reference /evidence/reference \
  --threads 1 --arm ordinary --warmup --resources /evidence/ordinary-resources
math_compaction --model-dir /models --reference /evidence/reference \
  --threads 1 --arm compact --warmup --resources /evidence/compact-resources
```

Every timed arm preloads actual production accepted strings before the timer and validates complete accepted UTF-8 using the existing Reader buffers. Warmup executes all engine sessions, then the selected arm once.

`cohort_ns` includes encoder, cache initialization, fixed decoder state
initialization, all decoder calls, gathers, input metadata, detokenization/output validation and final cache release. Numerical verification is excluded from timed runs.
`decoder_ns` starts after preparation. Model/session loading, reference input
file reads and page preprocessing are outside both boundaries. Gate runs mark
`timing_valid=false`; they include oracle calls and diagnostic I/O. Container
memory and CPU resource snapshots still include process setup and warmup and must be
reported with that scope. Neither timing measures Tika or a complete page.

This feature remains an experiment even if parity and a synthetic speed screen
pass. Preserve failed profiles and original tolerances. A slower candidate or a
numerical failure is a useful result; neither justifies changing the frozen
quality study or enabling compaction in production.

The [initial native correctness evidence](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/math-compaction/2026-10-18/README.md)
passes both CPU profiles with the required real shrink trajectory and direct
production output checks. The later [paired study](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/validation/math-compaction/2026-10-18/paired-study/README.md) completed forty timed arms and all six fresh reference/gate roles. One-CPU cohort ratios were consistently lower; two-CPU ratios ranged from 0.6064 to 1.2753, with three of ten compact arms slower. The feature remains opt-in. Gate timings are invalid for performance conclusions. The finite
[`measure-math-compaction.sh`](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/scripts/measure-math-compaction.sh) controller
creates fresh scratch containers, checks the exact image source identity,
regenerates references and gates, then runs all ten alternating paired
repetitions per CPU profile. Run it only after other local builds and
measurements stop. It fixes affinity to cores 0 and 0,1 for this host's study and
retains every started attempt, failure, resource snapshot and termination state.

The controller uses only Docker's default cgroup placement. Each process waits
at a bounded startup barrier while the host verifies its exact container ID,
owner label, immutable image, live host PID, Docker leaf and every CPU/memory
ancestor. The shared read-only helper is
[`cgroup-admission.sh`](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/scripts/lib/cgroup-admission.sh). Only after admission
does the host release the process to load models and run native work. Docker
calls and cleanup have finite deadlines; failed ownership, capture, stop or
removal is an explicit failure with retained evidence. `--gates-only` runs the
same host admission/reference/numerical path without collecting timed arms.
The diagnostic's `--record-reference` mode uses actual ordinary
`Engine::recognize` on the embedded pinned fixture, keeping reference capture
inside the same scratch binary and admission contract.

Before running the controller, prepare and retain a caller-locked build receipt:

```sh
scripts/prepare-math-compaction-receipt.sh "$SOURCE_COMMIT" "$EXPORTED_ELF" \
  "$NATIVE_RELINK_DIRECTORY" "$MODELS" "$BUILD_LOG" "$NEW_RECEIPT_DIRECTORY"
scripts/measure-math-compaction.sh "$IMAGE" "$MODELS" "$NEW_EVIDENCE_DIRECTORY" \
  "$NEW_RECEIPT_DIRECTORY" "$PRINTED_RECEIPT_SHA256" --gates-only
```

The receipt retains the exact prospective protocol, Cargo lockfile, native tool
lock, static archive and glibc manifests, native link archive hash, model lock
and verified file hashes, source/recipe archive, link environment and build log.
It declares a local relink from pinned recovered archives. It does not attest a
fresh native build. The controller checks the caller's locked receipt hash,
copies every material into evidence, verifies actual model files, copies the
actual `/math-compaction` ELF from its owned immutable container before startup,
and compares its SHA-256 with the receipt. Raw ELF inspection must show no
interpreter or dynamic library dependency. All later roles use that same image
without executable bind mounts. Raw topology and an explicit admission check
require CPU 0 and CPU 1 to be online, distinct physical cores in the same socket
and L3 domain before native work. Remove `--gates-only` only when resources are
quiet and the timing study is ready; the locked receipt and protocol are the
same for correctness and timed roles.

After all forty timed arms and cleanup succeed, generate the descriptive paired
report with:

```sh
bash scripts/analyze-math-compaction.sh "$NEW_EVIDENCE_DIRECTORY" "$PRINTED_RECEIPT_SHA256"
```

The analyzer independently admits the caller-locked material and executed ELF
hashes, producing source/protocol/image identity, both ordinary production
references and all four numerical gate reports before accepting timed data.
Host load, process identities, CPU accounting and pressure are retained before
and after the complete controlled run. These host counters are contextual and
cannot replace the per-arm cgroup measurements.

The report keeps every pair and summarizes the ten paired differences and ratios
at each CPU budget with their minimum, median and maximum. Cohort and decoder wall
times are separate from the recorded resource snapshot interval. The fields named
`full_arm_*` retain their original names, but CPU/throttling start after CLI setup
and host admission and end before the remaining final resource reads and process
exit. Between those reads they include model loading/verification, all-session
and selected-arm warmup, reference preparation, reporting and engine teardown.
Memory peak is the cgroup high-water mark when read. These are not literal whole
process lifetime measurements and cannot be attributed to the timed decoder alone.
No population confidence interval, production page
latency or Tika improvement follows from this synthetic experiment.

The four Criterion cases measure one hot self-cache K or V tensor gather/control
operation at the observed shrink steps. The same arena is reused without reset
between iterations. They exclude the other self cache, cross caches, mask,
tokens, input views, ORT, page preprocessing and Tika. An identity row prefix
still requires copies in later
layers when the logical batch stride shrinks. For three layers, eight heads and
head width 24, the expected movement of one self cache is:

| Shrink | Step | Moved layer-row blocks | Bytes |
| --- | ---: | ---: | ---: |
| 6 to 4 | 8 | 12 | 73,728 |
| 4 to 3 | 23 | 6 | 105,984 |
| 3 to 2 | 27 | 5 | 103,680 |
| 2 to 1 | 32 | 3 | 73,728 |

The sum is 357,120 bytes per self cache, matching the actual native count of
714,240 for both. The same 26 layer-row moves with memory length 50 explain
1,996,800 bytes for both cross caches. Mask and token copies add 1,200 and 48
bytes, respectively. Cross-cache, second self-cache, mask and token work belongs
to the complete native cohort measurement; the microbenchmark does not represent
those combined costs.
