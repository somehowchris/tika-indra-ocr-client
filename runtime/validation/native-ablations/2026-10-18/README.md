# Native one-factor ablations: retained measurements

All **128 fresh measured processes / 4,096 recognitions** and the four preceding
native gate roles completed successfully. Independent admission checked all 32
gate commands, every measured request, accepted UTF-8, actual resources, live host
PID/cgroup identities, exit and cleanup. All four candidates fail the frozen
conservative promotion rule. Thin LTO, spinning on and batch cap 64 remain the
baseline; these diagnostic results do not change a production setting.

Four fresh process pairs ran for each CPU/fixture/comparison in `AB, BA, BA, AB`
order. Each process warmed all enabled sessions and its selected fixture before
32 recognitions. Ratios below are candidate median latency divided by its paired
baseline median. Each table entry summarizes **four process ratios**, without
pooling requests across processes.

| CPUs | Fixture | Candidate | Ratio min / median / max | Slower candidate medians |
| ---: | --- | --- | --- | ---: |
| 1 | TIFF | fat LTO | 1.0010 / 1.0019 / 1.0061 | 4/4 |
| 1 | TIFF | spinning off | 0.9999 / 1.0021 / 1.0159 | 3/4 |
| 1 | TIFF | batch cap 1 | 0.9831 / 0.9979 / 1.0012 | 1/4 |
| 1 | TIFF | batch cap 8 | 0.9949 / 0.9984 / 1.0029 | 1/4 |
| 1 | math | fat LTO | 0.9975 / 1.0058 / 1.0191 | 3/4 |
| 1 | math | spinning off | 0.9817 / 1.0006 / 1.0173 | 2/4 |
| 1 | math | batch cap 1 | 0.9940 / 0.9984 / 1.0098 | 1/4 |
| 1 | math | batch cap 8 | 0.9917 / 0.9994 / 1.0191 | 2/4 |
| 2 | TIFF | fat LTO | 0.9325 / 0.9638 / 0.9979 | 0/4 |
| 2 | TIFF | spinning off | 0.3413 / 0.3478 / 0.3675 | 0/4 |
| 2 | TIFF | batch cap 1 | 0.9681 / 1.0745 / 1.0897 | 3/4 |
| 2 | TIFF | batch cap 8 | 0.9294 / 1.0047 / 1.0360 | 2/4 |
| 2 | math | fat LTO | 0.9676 / 0.9888 / 1.0098 | 1/4 |
| 2 | math | spinning off | 0.4888 / 0.4929 / 0.5071 | 0/4 |
| 2 | math | batch cap 1 | 1.1526 / 1.2048 / 1.2299 | 4/4 |
| 2 | math | batch cap 8 | 0.9647 / 0.9946 / 1.0152 | 2/4 |

Spinning off produced substantially lower paired median latency in both two-CPU
fixtures, while the one-CPU medians were approximately unchanged. This is a
useful candidate for a separate quota-specific representative-workload study.
These measurements do not identify the mechanism or establish a production/Tika
benefit. Fat LTO, small batch caps and spinning changes have different effects
across quotas and fixtures; reporting a pooled average would conceal those
tradeoffs. In particular, batch cap 1 increased all four two-CPU math medians,
with a median paired ratio of 1.2048.

The prespecified rule requires **every** pair across both fixtures and quotas to
have median ratio <= 0.95, within-process p95 ratio <= 1.05 and phase peak-memory
ratio <= 1.10, with all gates passing. No candidate satisfies it. The tails in
`paired.json` are nearest-rank summaries of 32 observations inside each process:
the 16th, 31st and 32nd ordered values give p50, p95 and p99. They are not
population p95/p99 estimates. Four process pairs provide descriptive replication;
no request-level independent confidence interval or general accuracy claim is
made. The complete observed ratios and resource values remain available,
including neutral and regressive results.

`phase_cpu_usage_us` and `phase_throttled_us` describe the recorded cgroup
snapshot interval. The before read occurs during initial phase attestation,
before the host-admission wait and tool verification. The after read occurs in
the exit trap after model verification/loading, all-session and selected-fixture
warmup, recognitions, reference comparisons, journal/output writes, final output
comparison and engine teardown. Initial container/shell setup and the remainder
of the final snapshot/exit fall outside that CPU interval. `memory.peak` is the
cgroup high-water mark at its read. These resource fields are neither warmed
recognition-only costs nor literal whole-process lifetime measurements. Already
cached model pages can be attributed to a different cgroup.

The study ran from 2026-10-18 23:04:18 to 23:24:06 UTC on the Ryzen 9 7950X3D
host, with exact total one/two-CPU CFS quotas and affinity `0` / `0,1`, 3 GiB
memory, zero swap, 128 PIDs and an 8 MiB stack. Every live Docker leaf and all
CPU/memory ancestors were admitted before model work. All containers used a
private cgroup namespace, read-only bundle/models/root filesystem, no network,
no capabilities and no-new-privileges. The bundle was copied into an individually
owned local volume; host permissions were unchanged and every owned container
and volume was removed. No perf sampling ran during these ablations. The
1088 MiB bounded memlock limit is inherited from the shared profile controller;
it is a limit, not allocated or resident memory.

Local builds and other task-owned inference were held during this window.
Unrelated host workloads remained present. Raw host load/process/CPU snapshots
are retained: one-minute load changed from 1.34 to 4.52. These contextual host
measurements do not establish CPU exclusivity or explain individual ratios.
Affinity `0` / `0,1` and its actual kernel enforcement are admitted here; this
ablation controller did not itself record a per-cell physical-core/L3 topology
check, so no stronger topology-admission claim is made.

The producing source was `ad048cefe35d950e82806fd56a65a91b84033848` for both
arms. The measured controller source was
`0a7dee13c8e2336c15a44887682bb588aee538a6`; its frozen inner runner came from
the producing source. Complete source/native archives and compiler identity
were byte-identical between arms. The effective diagnostic profile differed by
thin/fat LTO; release settings were unchanged. Executed diagnostic ELF SHA-256:

- thin: `9d091d447f32fa54ec12dab189e7dd77d868296d67a2ec88a565cb7db2534477`
- fat: `f36d064961cf55219a4f9c71269ead02dabd61d4a8a2aeba226de6284158fdc6`

The immutable tools image was
`sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142`.
`admission.json` records the independently rehashed full bundle/relink manifests,
static ELF inspection, source/native archive/compiler hashes and all fourteen
verified model files at revision `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`.
The 149-file execution-source closure also matched the clean producing checkout.
These are symbol-preserving diagnostic measurements, not signed release-image
or stripped-production-binary measurements.

The original first attempt is retained. It passed its four native gate roles
and completed the first measured recognition process, then failed postprocessing
with exit 5 because the jq resource parser rejected a trailing newline in
`memory.peak`. Its raw first-cell requests were not substituted into this run.
The narrowly scoped parser correction preceded the complete fresh second run;
source, protocol, settings, iteration count and measured executables stayed
unchanged. `failed-first-attempt/` and `failed-first-window/` in the portable
archive retain that status, raw cell and error. The second run includes all
128 new processes without trimming or replacing inconvenient observations.

`raw-records.tar.gz` contains 10,750 portable raw files: all 132 executed phase
journals, commands, request/output bytes, before/after limits/counters, complete
container/host PID/ancestor identities, tool checks, seed/volume ownership and
cleanup records; both initial bundle admissions; exact executed controller
sources; and both attempt windows. `original-run-files.sha256.gz` binds all
18,809 original completed-run files, including repeated large inventories
omitted from the portable archive. Binary, source/native archive and model
payloads remain outside Git, bound by their original manifests and the explicit
large-material verification receipt. Portable replay does not rehash omitted
large payloads.

From the repository root, replay portable admission and its eight rejection
checks without Docker or native dependencies:

```sh
python3 scripts/test-native-ablation-evidence.py \
  runtime/validation/native-ablations/2026-10-18
python3 -O scripts/test-native-ablation-evidence.py \
  runtime/validation/native-ablations/2026-10-18
```

All nine checks passed in both modes. They verify portable file bytes before
admitting the complete study, then reject modified quota, OOM, output, request
count/settings, live host PID, cleanup and paired arithmetic records.

Original local evidence:

- `/vol/md1/tika-indra-artifacts/native-ablations-v2-ad048ce-20261018`
- `/vol/md1/tika-indra-artifacts/native-ablation-window-v2-ad048ce-20261018`
- `/vol/md1/tika-indra-artifacts/native-ablations-ad048ce-20261018`
- `/vol/md1/tika-indra-artifacts/native-ablation-window-ad048ce-20261018`
- `/vol/md1/tika-indra-artifacts/ablation-build-ad048ce/{thin,fat}`
