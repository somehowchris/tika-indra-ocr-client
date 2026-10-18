# Native profiling and control-path measurements

The [workflow](../.github/workflows/native-profile.yml) builds a separate static
diagnostic bundle, runs native gates and all 46 prescribed Criterion cases, and
captures warmed OCR under **total one-CPU and two-CPU limits**. The complete
[protocol](../runtime/validation/native-profile-protocol.json) is an input to
the run. No new timing, flamegraph or strengthened solver result is claimed
until the corresponding hosted artifacts exist and pass review.

This follow-up restores source lost during the interrupted workspace session.
Its protocol records the previous unpublished protocol hash and the recovery
scope. Both synthetic fixture outputs are restored against hashes already
published in the earlier reference comparison. They are implementation
fixtures, excluded from the multilingual quality corpus and its English share.

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
Each `enable` and `disable` requires exactly `ack\n` within five seconds. Stale,
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

Denied access, nonzero commands, incomplete output, fewer than 100 samples,
any reported loss, or a folded-stack/sample-count mismatch fail the profile.
Raw traces, command lines, exit codes, kernel policies, actual resources and
stderr are retained on failure. No missing hardware event or failed FIFO run is
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
bundle. At least one reviewer must inspect each complete SVG and PNG for
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
