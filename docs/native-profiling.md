# Native profiling and control-path measurements

The [workflow](../.github/workflows/native-profile.yml) builds a separate static
diagnostic bundle, runs native gates and all 46 prescribed Criterion cases, and
captures warmed OCR under **total one-CPU and two-CPU limits**. The complete
[protocol](../runtime/validation/native-profile-protocol.json) is an input to
the run. The frozen source-`2193324` [completed baseline](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/tests/evidence/2026-10-18-native-profile-baseline/README.md) now passes independent normal/optimized admission: all 46 Criterion cases at each CPU cap, four captures with zero lost samples, matching process IDs and complete resource/cleanup gates. The four original figures were independently viewed and raw perf streams replay exactly. Unknown math frames and existing host activity, including a brief semantic search during two-CPU Criterion, remain disclosed. These are descriptive warmed synthetic-fixture profiles, not whole-Tika performance or OCR-quality results.

Earlier preflight, transport and profiler failures remain preserved below and in the retained baseline packet. A successful later run does not convert those attempts into accepted sampling evidence. The separate [strengthened arithmetic proof](batch-proof.md#current-verified-run) passed.

This follow-up restores source lost during the interrupted workspace session.
Its protocol records the previous unpublished protocol hash and the recovery
scope. Both synthetic fixture outputs are restored against hashes already
published in the earlier reference comparison. They are implementation
fixtures, excluded from the multilingual quality corpus and its English share.
The [original protocol](../runtime/validation/audit/native-profile-original-0001.json)
remains byte-exact. Its [prospective amendment](../runtime/validation/audit/native-profile-amendment-0001.json)
changes the incorrect control/target setup and adds sample identity admission;
the selected cases, resource limits, repetitions and acceptance thresholds stay
the same.

## Execute

Start from the producing repository revision and a clean working tree:

```sh
git diff --exit-code
docker buildx build --platform linux/amd64 --file docker/native.Dockerfile \
  --target engine-profile-export --build-arg BUILD_CPUSET=0,1 \
  --build-arg SOURCE_COMMIT="$(git rev-parse HEAD)" \
  --output type=local,dest=native-profile-bundle .
```

Use CPU IDs allowed on the builder. Compilation affinity is not a measurement
quota. The GNU archive layer is shared with the service build; only the Rust
profiling profile and diagnostic executables are linked again. The export
includes unstripped OCR, exact native/unit/FIFO test binaries, four Criterion
executables, standalone allocation gates, frozen fixtures, build identities,
and the **complete** native/Rust/glibc relink material bundle. A relative hash
manifest covers all of these files. The material identity must name this
profiling executable's hash.

The workflow installs host `perf` and the SVG renderer, records the exact host
packages and kernel, then builds [Inferno 0.12.8 at its pinned commit](https://github.com/jonhoo/inferno/tree/929be68e19a27da062623c6da5255e8b96faebbc)
with its upstream Cargo lock. On an equivalent Linux host with systemd and
cgroup v2, supply explicit paths to those tools and an already verified model
cache:

```sh
sudo env PERF=/absolute/path/to/perf \
  INFERNO_COLLAPSE=/absolute/path/to/inferno-collapse-perf \
  INFERNO_FLAMEGRAPH=/absolute/path/to/inferno-flamegraph \
  bash scripts/profile-native.sh \
  "$PWD/native-profile-bundle" "$PWD/model-cache" "$PWD/profile-evidence/new-run"
```

The output directory must be new. Each gates, Criterion, TIFF or math phase
owns a fresh systemd cgroup. The shell, OCR process, profiler and analysis tools
share `cpu.max = N × 100000 / 100000`, N permitted CPUs, 3 GiB memory, zero swap,
and 128 tasks. The runner checks its own process membership, effective cpuset,
scheduler affinity, ancestor limits and 8 MiB main-thread stack. Absolute phase
and command deadlines preserve a finite stop path. Cleanup targets only the
unit whose ownership description matches this invocation.

Model acquisition and identity checks warm the filesystem cache before the
measured phase. This is cache-warmed execution, not cold-storage startup. File
pages can remain charged to a different cgroup; retained `memory.peak` and
`memory.events` describe the actual phase accounting, not an isolated cold-cache
memory footprint.

## What is measured

| Gate or measurement | Scope |
| --- | --- |
| Native unit/acceptance | Actual linked preprocessing, tensor-bit equivalence, TIFF text, errors and optional decoder behavior |
| Rust allocation gates | Exact zero allocation/reallocation/deallocation churn in the planner, old/new tensor scan, core control path and trace-parent parsing |
| Model allocation observations | Counts for 100 real verification passes per model set; reported as observations, with no zero-allocation threshold |
| Planner Criterion | 24 combinations of line count, row cap and text/math policy |
| Tensor-scan Criterion | Eight prescribed shapes, each with previous maximum scan and production `BatchPlan` accounting |
| Other Criterion | Three core operations, two actual file-verification cases, one fixed-array W3C parent parser |
| OCR timing | Three 64-request blocks per fixture: unrecorded, recorded, unrecorded |
| Native flamegraphs | User-space software CPU-time samples from the warmed repeated OCR loop |

Criterion uses 100 samples, a one-second warmup and a three-second measurement
target. The runner validates the complete case identity set and sample arrays;
an omitted or duplicated case cannot satisfy the gate. Timing uses the recorded
allocator: mimalloc for OCR and engine/core/model benchmarks, System for the
trace-parent benchmark. Allocation instrumentation uses System and observes only
Rust allocator calls in the stated region. It is not a count of all native
ONNX Runtime or OpenCV allocations. Standalone executables avoid libtest
background allocations contaminating a process-wide counter.

The tensor account is a bounds integration, not an asserted optimization. It
replaces the old maximum-width variable after unchanged native row selection.
The runner retains each Criterion mean and confidence interval, plus a
descriptive ratio between the two means. It does not treat that ratio as a
causal speedup interval. [The arithmetic proof](batch-proof.md) and native bit
comparison remain separate evidence.

## Sampling boundaries and rejection rules

`ocr --warmup` executes all enabled sessions and the selected fixture before the
measured loop. Output capacity and the stats file are prepared before enabling
sampling. The controller uses Linux perf's documented
[`--delay=-1` and FIFO protocol](https://github.com/torvalds/linux/blob/v6.8/tools/perf/Documentation/perf-record.txt).
Each `enable` and `disable` requires exactly **five bytes, `ack\n\0`**, within
five seconds. This matches the actual perf 6.17.13
[C string definition](https://github.com/gregkh/linux/blob/v6.17.13/tools/perf/util/evlist.h)
and [write of its `sizeof`](https://github.com/gregkh/linux/blob/v6.17.13/tools/perf/util/evlist.c),
which includes the terminating NUL. Stale,
malformed, truncated or already queued extra bytes fail. If OCR and disable both
fail, both errors are retained and the measurement boundary is explicitly
unconfirmed. The nine real FIFO tests exercise these paths without a model or
invented profiler response count.

Sampling uses `cpu-clock:u` at 99 Hz, strict frequency admission, 16 KiB DWARF
stack captures, a 32 MiB mmap area and a 256 MiB trace ceiling. It includes the
warmed loop's stats writes and control boundary overhead. It excludes model
verification, initialization, fixture warmup, the final output-byte comparison
and teardown. Per-request latency stops before its JSONL write, so timing and
sampled regions have this small, disclosed difference.

The `perf record` workload inherits events into its own threads. The command
does not pass `--cpu`, `--all-cpus` or `--per-thread`: the
[kernel documentation](https://docs.kernel.org/userspace-api/perf_ring_buffer.html)
distinguishes default workload inheritance from `--cpu`, which samples all
tasks on selected CPUs. External quota and affinity still constrain the OCR
process. The CLI records its actual PID before enabling sampling; every sample
header must identify that same PID and a valid thread ID. The admission helper
rejects an incomplete request journal, foreign processes or events, malformed
records, missing/ambiguous loss counters and inconsistent folded weights.
Its offline fixtures test those rejection rules without presenting synthetic
records as measured performance.

Denied access, nonzero commands, incomplete output, fewer than 100 samples,
any reported loss, or a folded-stack/sample-count mismatch fail the profile.
Raw traces, command lines, exit codes, kernel policies, actual resources and
stderr are retained on failure. Bounded command/phase excerpts also appear in
the job log, so the reason is visible without downloading a large archive.
No missing hardware event or failed FIFO run is
silently replaced with another measurement. Software CPU-time sampling does
not measure IPC, vector instruction counts or the benefit of an AVX kernel.

For local debugging, a startup-inclusive capture can run the same executable
under `perf record` without the FIFO arguments. Label it **whole-process,
startup-inclusive** and keep it outside this controlled comparison. An
unrestricted shell also has no one/two-CPU attestation. This fallback is not a
substitute for the workflow's warmed, resource-attested gate.

## Review the evidence

The workflow exports `perf.data`, full report/script text, every folded stack,
deterministic SVG and rendered PNG, unrecorded/recorded timings, all Criterion
sample files, allocation observations and the complete diagnostic material
bundle. Evidence and relink materials are separate compressed Actions artifacts;
the complete source/material hash manifest is retained. At least one reviewer
must inspect each complete SVG and PNG for
readability, clipping, unknown frames, dominated stacks and a meaningful warmed
OCR region. The machine record deliberately leaves `visual_review` pending;
creating an image does not complete that review.

The unstripped Rust binary retains DWARF line tables. Native archives can expose
function symbols without C++ source lines; report that limitation alongside
unknown-frame counts. Inspect dominant CPU paths first, propose one bounded
change with a prespecified equivalence/performance gate, and rerun only the
affected comparisons. Passing these two synthetic profiles does not establish
representative document latency, cross-request continuous batching, optimal
thread counts, fat-LTO benefit or that every optimization option is exhausted.

## Retained partial run: 18 October 2026

[Run 39150937754](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39150937754)
executed merge commit `0fe8d1f1ec768385261b529fcd16b898df7e6467`, from PR head
`745efa8a11def7812964959f6c9d8bba957e63fd`. The compact
[receipt](../runtime/crates/indra-engine/tests/evidence/profile/partial-39150937754.json)
binds these numbers to the original archive and the successful read-only
[recovery run](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39157989267).
Recovery extracted existing evidence; it did not repeat measurements.

At the actual one-CPU limit, 26 native unit tests, three native integration
tests, nine FIFO tests using the old incorrect ACK fixture, and two CLI tests
passed. The core, engine and telemetry allocation gates recorded zero in all
six Rust allocator counters. The model verification observation recorded 3,300
allocations and deallocations, 57,400 allocated/deallocated bytes, and zero
reallocations in each 100-pass text/math arm. That older executable omitted
`bytes_reallocated`; the missing counter is not inferred to be zero.

All 46 Criterion cases completed. The eight tensor-scan comparisons measured
the following point means in nanoseconds; the receipt retains their individual
95% Criterion intervals. Ratios of point means are descriptive, with no claimed
paired or causal interval.

| Shape | Previous maximum scan, ns | Proved bounds account, ns | Added time, ns |
| --- | ---: | ---: | ---: |
| Text 1 × 8 | 1.735 | 4.299 | 2.564 |
| Text 1 × 4096 | 1.807 | 4.296 | 2.488 |
| Text 8 × 64 | 5.442 | 17.855 | 12.413 |
| Text 8 × mixed widths | 5.477 | 18.208 | 12.731 |
| Text 62 × 657 | 49.344 | 123.105 | 73.761 |
| Text 63 × 649 | 48.844 | 125.649 | 76.805 |
| Text 64 × 640 | 49.954 | 126.256 | 76.303 |
| Math 64 × 4096 | 49.365 | 135.722 | 86.357 |

This result identifies a measurable cost of the production safety boundary:
approximately 2.38–3.32 times the previous scan's tiny cost. It establishes
neither an OCR speedup nor a meaningful end-to-end regression. A later
optimization decision needs complete OCR profiles and paired resource-limited
measurements, while preserving the proved capacity guard and exact tensors.

The initial 64 warmed, unrecorded TIFF requests all returned the exact 33-byte
reference. Nearest-rank median latency was 49.254 ms and p95 was 52.051 ms
(range 48.033–70.937 ms). These are descriptive repetitions of one synthetic
fixture. The TIFF phase recorded a 138,711,040-byte cgroup `memory.peak` and no
OOM event; model pages had already been warmed outside that cgroup, as disclosed
above.

The recorded block then failed before its first request: the client consumed
`ack\n` and rejected perf's legitimate trailing NUL. Its journal was empty and
the phase exited 1. Independently, the original `--cpu` selector would have
included unrelated processes. Both defects are corrected prospectively.
Neither math profiling nor the two-CPU phases ran, and no flamegraph from this
attempt is accepted. Corrected real sampling, complete one/two-CPU results and
visual review remain required.

The diagnostic Inferno build applies one recorded patch to upstream
[929be68](https://github.com/jonhoo/inferno/blob/929be68e19a27da062623c6da5255e8b96faebbc/src/collapse/perf.rs#L505):
the leading-parenthesis rule now preserves qualified `(anonymous namespace)::`
C++ function symbols. Process-name markers retain the upstream skip behavior.
Original and patched source archives, patch bytes and hashes are retained before
compilation. The sampling verifier and thresholds are unchanged: a header-only
sample still makes raw and collapsed counts differ and rejects that profile.

Historical failed run 39233052354 contained 21 single-frame anonymous-namespace
Mlas samples that upstream Inferno omitted (1010 raw, 989 collapsed). Replaying
those retained bytes with the corrected tool is a derived stack conversion, not
a new profile or current-release performance acceptance. Historical TIFF runs
39233070654 and 39233005000 contain actual empty stacks and remain rejected.

The [tooling replay receipt](evidence/inferno-anonymous-symbol-20261019/receipt.json)
binds the source producer, both source archives, compiler artifacts, bounded
owner, retained failed setup attempt and actual historical merge producers.


## Idle-session CPU policy

Math loads ten ONNX sessions, each with its own intra-operation pool. Previously,
inactive pools could keep spinning while another session needed the same CPUs.
The runtime now sets `session.force_spinning_stop=1` only when intra-operation
spinning is enabled and the thread count exceeds one. Active calls retain
spinning; one-thread and explicitly nonspinning execution retain their original
native Run path. This changes scheduling, with the same FP32 models, math,
operators, batching and optimized release profile.

The [ort performance guide](https://ort.pyke.io/troubleshooting/performance)
recommends checking graph optimization, providers, I/O binding and precision.
Graph optimization is already `All`; borrowed CPU tensors and native-owned math
cache outputs avoid Rust tensor-payload copies. GPU transfer savings do not apply
to this CPU provider. INT8, FP16 and graph/provider substitutions require separate
fidelity evidence. The pinned [ORT implementation](https://github.com/microsoft/onnxruntime/blob/402979ac1bef89dcd55add41eaa7277c407599eb/onnxruntime/core/session/inference_session.cc#L3646)
disables idle-pool spinning after the last Run and restores it for the next.

Two separate whole-Tika diagnostics compare immutable source `689ae88` with only
patch `8982e26a04b19e0694c4a0d51214644ce1fa1d86346b9a1895e3c07eb2eb0e51`.
Each uses four fresh paired processes per quota, three known inputs, math on,
a shared 1/2-CPU and 3 GiB/zero-swap budget including the Rust load generator,
and fixed counterbalanced order. The longer replication uses 12 timed requests
per input per process; CPU windows include client setup, one warmup and 36 timed
responses, excluding server startup and cleanup. These are CPU milliseconds per
response over that window, not isolated native kernel cycles.

| Quota | Baseline → guarded CPU ms/response | Mixed mean latency ms | Successful responses/s |
|---|---:|---:|---:|
| 1 CPU | 150.261 → 148.542 | 144.670 → 142.937 | 6.895 → 6.983 |
| 2 CPUs | 326.897 → 171.298 | 165.827 → 92.947 | 6.021 → 10.733 |

All 592 replication responses match the preceding output witnesses byte for
byte. The first, shorter experiment independently observed 44.6% lower two-CPU
cost; replication observed 47.6%. One-CPU changes are small: +0.4% CPU cost in
the first experiment, −1.1% in replication, with individual latency regressions
retained. Two-CPU system CPU rises 9.9% in replication even as total CPU falls.
The replication plan SHA-256 is `d1a920b08eb60564de864d15740148ff66207368b99f2105241efcbd6b73ee73`.
The three inputs are two laboratory oracle crops and one synthetic math fixture;
these results do not establish representative accuracy or universal speedup.

Four separate native-only math profiles retain 742/742/906/2189 samples, zero
loss and 1/2/1/0 samples with unknown frames (one-CPU baseline/guarded, two-CPU
guarded/baseline). All four output/resource/cleanup gates pass. In the two-CPU
captures, `SpinPause` occupies 383/2189 baseline samples versus 33/906 guarded
samples; useful MLAS and OpenCV kernels remain. These software samples are
neither hardware IPC measurements nor an independent whole-Tika speedup test.
Full signed-candidate and multilingual validation remains required.

A direct Python/Rust diagnostic exposed another configuration difference:
Python did not request `session.force_spinning_stop`; its pinned
[ORT default is `0`](https://github.com/microsoft/onnxruntime/blob/402979ac1bef89dcd55add41eaa7277c407599eb/onnxruntime/core/session/inference_session.cc#L544).
A separate COPY-only Python derivative explicitly matched all three spinning
settings: intra/inter-operation spinning `1`, and force-stop `1` with two threads.
Four fresh pairs in AB/BA/BA/AB order retained math, FP32, one worker, two threads,
batch cap 64 and a common two-CPU/3 GiB/zero-swap parent containing Tika, OCR and
client. The same two known Portuguese oracle crops and synthetic math page were
used, with twelve technical repetitions per case and process.

| Matched-spinning metric | Python | Guarded Rust |
|---|---:|---:|
| CPU ms/response, including warmup | 181.220 | 168.259 |
| Mixed timed responses/s | 10.073 | 10.918 |
| Mixed mean latency, ms | 99.042 | 91.352 |

Pooled CPU was 7.15% lower and throughput 8.39% higher for Rust. CPU includes
client setup and warmups (148 responses/condition), excluding startup/cleanup;
timed throughput excludes warmups. All 296 bodies matched prior witnesses.
Aggregate CPU improved in all four pairs, but hops-crop mean latency regressed
1.94% and 3.08% in two pairs, and pooled Tika CPU increased 4.15%. Charged parent
lifetime peaks were Python 491–503 MB versus Rust 576–602 MB; these include startup
and cache charges and are neither RSS nor per-page allocation.

The earlier unmatched comparison remains separate: 355.495 → 169.127 CPU ms/response
(52.4% lower), 179.006 → 90.750 ms mean latency and 5.579 → 10.993 responses/s, with
296 exact bodies, Tika CPU +2.398%, one OCR system-CPU pair +0.416% and Rust charged
peaks 17.1–24.0% higher. It cannot establish a language-performance advantage.
Comparing these two packets also does not isolate one setting's causal effect.

Both diagnostics use unsigned Python base `60c849a` and guarded Rust base `689ae88`
plus patch `8982e26a…`, rather than a current signed release. Native builds,
packaging, other settings and Python memory-pattern `true` versus Rust `false`
still differ. The three inputs and previous performance were known; representative
accuracy, isolated language causality, universal improvement and maximum possible
performance remain unestablished. Production Python defaults and the main six-arm
study are unchanged. Independent raw audits pass 444 unmatched and 492 matched
numerical comparisons within `1e-12`, with exact integers and raw latency arrays.
The original unmatched analyzer rejection and its identity-only correction remain.
Local packet manifests are `quick-python-guarded-rust-2cpu-12rep-20261019-gq9x4nwy`
(`df770eaae104b3437890f8f618ae8f01eefad3250f60afd2fc181a663e98217a`)
and `quick-matched-python-guarded-rust-2cpu-12rep-20261019-bjha_d_c`
(`90a24076f0b2bacda4967adee09d362e02cb66e1810624ca6690387a3eb72038`).
