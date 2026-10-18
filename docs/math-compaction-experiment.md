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
  --threads 1 --arm compact --gate
```

The gate rejects a trace containing more than one math cohort. It checks the
replayed ordinary encoder, mask, cross caches, logits and token/stop snapshots
against the trace from `Engine::recognize`. An independent ordinary stream stays
at six rows. Before and after every candidate step, the gate compares live rows
by their original cohort index, including both self caches, both cross caches,
mask, logits, tokens, probability history, stop reason/step, token history,
confidence acceptance and decoded UTF-8. Floats must be finite and satisfy the
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
Every moved block is checked against its still-intact source bits immediately.
Independent sentinel tests construct expected values from immutable original
coordinates, including multiple layers, heads, cache time, repeated gathers,
signed zero, boundaries and invalid selections.

The allocation executable measures only prepared gather/control operations with
`stats_alloc` and the System allocator. Criterion measures the same copy and bit
verification core on reused arenas. Neither establishes whole-decoder zero
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
  --threads 1 --arm ordinary --warmup
math_compaction --model-dir /models --reference /evidence/reference \
  --threads 1 --arm compact --warmup
```

`cohort_ns` includes encoder, cache initialization, fixed decoder state
initialization, all decoder calls, gathers, bit checks and input metadata.
`decoder_ns` starts after preparation. Model/session loading, reference input
file reads and page preprocessing are outside both boundaries. Gate runs mark
`timing_valid=false`; they include oracle calls and diagnostic I/O. Container
memory and CPU evidence still include process setup and warmup and must be
reported with that scope. Neither timing measures Tika or a complete page.

This feature remains an experiment even if parity and a synthetic speed screen
pass. Preserve failed profiles and original tolerances. A slower candidate or a
numerical failure is a useful result; neither justifies changing the frozen
quality study or enabling compaction in production.
