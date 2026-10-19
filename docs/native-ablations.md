# Native one-factor ablations

This prospective study compares five configurations on the frozen synthetic
TIFF and math fixtures under total **1 CPU** and **2 CPU** limits. It is separate
from the [46-case/four-capture profiling baseline](native-profiling.md), the
multilingual quality corpus and whole-Tika measurements. The [completed measurements](../runtime/validation/native-ablations/2026-10-18/README.md)
retain all 128 processes and 4,096 requests, with four native gate roles passing.
No candidate meets the frozen conservative promotion rule. Two-CPU spinning-off
results warrant a separate representative-workload study; production defaults
remain unchanged.

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

The Docker fixture checks real resource admission, required native command flags,
intentional exit retention and owned-container cleanup. It emits no OCR timings
and is not performance evidence.
