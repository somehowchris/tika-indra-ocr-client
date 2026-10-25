# Native one-factor ablations

This prospective study compares five configurations on the frozen synthetic
TIFF and math fixtures under total **1 CPU** and **2 CPU** limits. It is separate
from the [46-case/four-capture profiling baseline](native-profiling.md), the
multilingual quality corpus and whole-Tika measurements. The [completed measurements](../runtime/validation/native-ablations/2026-10-18/README.md)
retain all 128 processes and 4,096 requests, with four native gate roles passing.
None of these historical four candidates met the frozen conservative promotion
rule or was promoted. The separate [guarded idle-pool policy](native-profiling.md#idle-session-cpu-policy)
retains spinning during active model calls and has its own measurements.

These measurements predate the Indra 1.1 engine. They ran on 2026-10-18 at
source `ad048ce` on the local Ryzen 9 7950X3D host, with the pre-1.1 model set
at revision `ed345c7f` (indra-ocr 0.3.2 pipeline, without LM beam search or
dual-read routing), in symbol-preserving `profiling` executables rather than
the stripped `release` binaries, and before the guarded idle-pool spinning stop
existed. They were not repeated with Indra 1.1 (`bd02167f`) or on the hosted
runners that measured the frozen study. Batch cap 64, spinning on and thin LTO
are settings carried over unchanged into the 1.1 study, not optima measured on
1.1. Among retained evidence, the choice of thin over fat LTO in the release
profile of `runtime/Cargo.toml` rests on this diagnostic alone; the
[always-math amendment](../evaluation/audits/always-math-20261019/amendment.json)
also lists an unretained quick two-CPU diagnostic,
`quick-math-maxperf-2cpu-20261019-bsfc72zb`. No paired measurement covers the profile's
`overflow-checks = true`.

![All64 observed paired native process medians](../evaluation/figures/native-ablations-paired.png)

The [Seaborn renderer](../evaluation/report/native_ablations.py) checks every plotted value, category, position and color, including all four legend entries. The [separate visual receipt](../evaluation/figures/native-ablations-visual-review.json) binds the inspected PNG/SVG and renderer bytes.

The [frozen protocol](../runtime/validation/native-ablation-protocol.json)
compares the thin-LTO, spinning-on, batch-cap-64 baseline separately against fat
LTO, spinning off, cap 1 and cap 8. Each CPU/fixture/comparison has four fresh
process pairs in `AB, BA, BA, AB` order. Each process warms all enabled sessions
and the selected fixture, then performs 32 recognitions. The complete study has
128 measured processes and 4,096 requests, preceded by exact native gates for
both linked artifacts at both CPU budgets. The scheduling command exposes the
complete order before any execution:

```sh
bash scripts/ablate-native-docker.sh --schedule
```

Commit this source and protocol before building either arm. Export two complete
bundles from that same commit and pinned native closure:

```sh
revision=$(git rev-parse HEAD)
for lto in thin fat; do
  docker buildx build --platform linux/amd64 --file docker/native.Dockerfile \
    --target engine-profile-export --build-arg SOURCE_COMMIT="$revision" \
    --build-arg PROFILE_LTO="$lto" \
    --output "type=local,dest=native-ablation-$lto" .
done
bash scripts/ablate-native-docker.sh \
  native-ablation-thin native-ablation-fat model-cache ablation-evidence/new-run \
  indra-profile-tools:issue13
```

Supply the builder's permitted `BUILD_CPUSET` and the same native build/cache
arguments as the baseline build where required. Only
`CARGO_PROFILE_PROFILING_LTO` differs between the two arms. The serving release
profiles stay unchanged. These symbol-preserving diagnostic executables are not
stripped production-binary timings. Both bundles must identify their effective
LTO setting, exact source commit and diagnostic ELF; admission compares their
complete source archive, native link archive and compiler identity. Every
measured recognition must byte-match the frozen expected UTF-8 output.

The controller reuses the profile runner's private Docker namespace, exact
ownership cleanup, read-only source/bundle/model mounts, host ancestor checks,
3 GiB memory, zero swap, 8 MiB stack and finite phase deadlines. Ablations add no
capabilities and use no perf recorder. An interruption forwards termination to
the active controller and waits for its bounded cleanup. Failed cells remain in
the evidence directory; partial studies cannot produce `completion.json` or a
default decision. Output directories must be new.

Raw request JSONL includes latency, stage times, batch counts and valid/padded
columns. Per-process summaries retain nearest-rank p50/p95/p99, recognition rate from
the summed request latency, phase-snapshot
cgroup CPU time and throttling, and phase memory peak. Latency stops before the
reference check and JSONL write. The CPU snapshot interval starts during initial phase attestation and ends
in its exit trap. It includes the host-admission wait, tool checks, model
verification/loading, warmup, recognitions, reference checks, journal/output
writes and engine teardown; it excludes initial container/shell setup and the
remainder of the final snapshot/exit. It is not warmed-loop-only CPU cost or
literal whole-process lifetime. Cache warming and file-backed memory attribution limitations match the
baseline. Model verification can leave pages charged elsewhere.

`analysis.json` preserves every process pair and its latency/memory ratios. The
four pairs are descriptive process replicates, with no request-level independent
confidence interval. The prespecified conservative policy retains the baseline
unless every pair across both fixtures and quotas has at least 5% lower median,
no more than 5% p95 regression and no more than 10% higher phase peak memory, with
all correctness/resource gates passing. A candidate meeting this policy still
needs representative-workload confirmation. Neutral and regressive results must
be retained. No exporter/telemetry, ISA-isolation or optimization-exhaustion claim
follows from this study.

Run the control checks without native dependencies, then optionally exercise
both actual Docker CPU paths with an explicit no-timing executable fixture:

```sh
bash scripts/test-native-ablation-control.sh
bash scripts/test-native-ablation-control.sh --docker indra-profile-tools:issue13
```

The Docker fixture checks real resource admission at 1, 1.5 and 2 CPUs, required
native command flags, the warmed-loop counter exchange, intentional exit retention
and owned-container cleanup. It emits no OCR timings and is not performance
evidence. `tests/native-loop-counters.sh` runs the same counter exchange against a
stand-in client on real Linux FIFOs without Docker.

## Thread-policy study (Indra 1.1)

The [thread-policy protocol](../runtime/validation/native-thread-study-protocol.json)
was registered before any measurement. It compares the shipped two-CPU OCR setting
A (ONNX Runtime threads 2, OpenCV threads 2, spinning on) with B (2/1), C (1/1),
D (2/2, spinning off) and E (1/2) on the Indra 1.1 engine, with the language
models on for both fixtures and math for the math fixture. Each comparison runs
four fresh `AB, BA, BA, AB` process pairs of 64 warmed requests on both fixtures
at three placements:

| Placement | Quota | Cpuset | Deployment it represents |
| --- | ---: | --- | --- |
| `smt-siblings` | 2 CPUs | two SMT siblings of one core | the hosted runners' and sealed evaluation's two-CPU topology |
| `distinct-cores` | 2 CPUs | two physical cores | dedicated two-core hosts |
| `shipped-ocr-quota` | 1.5 CPUs | the SMT sibling pair | the shipped `compose.2cpu.yaml` / `deploy/overlays/2cpu` OCR partition |

That is 192 measured processes and 12,288 recognitions after native gates at one
and two CPUs. The controller reads `thread_siblings_list`, `core_id` and
`physical_package_id` from sysfs for every allowed CPU, picks both pairs and stops
before any container starts when the runner lacks one. The guest-reported topology
is recorded, not verified against the host's physical placement. Every output must
byte-match the fixture's reference text. The study has no `workers = 2`, LTO or
`overflow-checks` arm and does not re-test the release profile.

Per process the analysis keeps nearest-rank p50/p95/p99, per-stage p50 of
`stage_us`, phase CPU/throttling/peak memory, and warmed-loop counters: cgroup
`usage_usec`, `user_usec`, `system_usec`, `nr_periods`, `nr_throttled`,
`throttled_usec` and the voluntary and involuntary context switches of every OCR
thread. The controller takes those counters by answering the executable's
existing `--perf-control` FIFO exchange itself. It reads `cpu.stat` last before
acknowledging `enable` and first once `disable` arrives, so the counters bracket
the 64 measured requests and leave the snapshot's own work outside. No profiler
runs. A thread that exits inside the loop, or a second OCR process in the cgroup,
fails the cell; a thread that both starts and exits inside the loop is not counted.

The decision rule is per placement. A candidate is faster in a placement when all
eight of its pairs there have median ratio ≤ 0.95, and cheaper when all have
median ratio ≤ 1.00 and warmed-loop CPU per request ratio ≤ 0.85. It is promoted
when it is faster or cheaper and every pair keeps p95 ratio ≤ 1.05 and peak
memory ratio ≤ 1.10. No one-CPU result is required.

Engine pairs alone change no shipped file. A candidate selected in
`shipped-ocr-quota` becomes a proposal for `compose.2cpu.yaml` and
`deploy/overlays/2cpu/resources.yaml` only if none of its `distinct-cores` pairs
has median ratio above 1.05, because the shipped profile pins no CPUs. The
proposal changes those files only after a separately registered, paired
whole-Tika confirmation passes: the `rust-math-b64` arm through the evaluation
runner on one host at the shipped OCR 1.5 / Tika 0.5 CPU partition, with
byte-identical outputs, throughput not below A's and p95 at most 1.05 times A's.
The evaluation runner has no per-role partition mode yet, so that confirmation is
not part of this study. A candidate selected in either other placement is recorded
as an engine-level result, not as a whole-stack setting. The protocol also fixes
when per-session detector threads would be worth implementing, and reports the A
processes at 2 and 1.5 CPUs on the same cpuset side by side as an unpaired
description of the partition's cost.

Run it on a hosted runner only from a source whose engine already includes the
vectorised skew row sums, the fixed skew-warp and detector-mask buffers and the
language-model search's early exit. They change stages that the comparisons read,
so a result from an earlier engine source is re-run, not acted on:

```sh
gh workflow run native-profile.yml --ref <branch> -f study=thread-policy
```

Before the bundle link, the job runs the offline rule fixtures and resolves the
runner's sibling and distinct-core pairs from sysfs, so a runner without either
pair stops early. It then links the same profiling bundle as the profile and runs
the Docker controller fixture, `scripts/ablate-native-docker.sh --threads` and
`scripts/admit-native-thread-study.sh`. Admission rechecks every cell's exit,
cleanup, quota, cpuset, settings, journal and output hash, recomputes every process
summary and the decisions from the retained raw files, and prints the decision
table into the job summary. Evidence uploads as `native-thread-study-evidence`.
No result of this study has been recorded yet.
