Continuous batching fails the frozen performance screen and remains disabled by default.
All 240 cells completed, all 2,880 responses matched their same-layer reference
bytes, no container was OOM killed, and all owned containers were removed.
The native and Tika controllers intentionally returned 1 because each records
only its own layer; both assigned layers completed 120/120 with cleanup 0.
The enclosing serial window returned 0. This distinction remains in the raw files.

Ratios compare continuous with ordinary processing. Throughput above 1 is faster;
p95 and CPU ratios above 1 are worse. Each median uses ten paired fresh processes,
not independent request observations. The memory column is the worst paired ratio.

| Layer | CPUs | Concurrency | Throughput median | p95 median | CPU/success median | Memory ratio max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| native | 1 | 1 | 1.041 | 0.967 | 0.961 | 1.074 |
| native | 1 | 2 | 1.044 | 1.208 | 0.957 | 1.102 |
| native | 1 | 3 | 1.019 | 1.158 | 0.982 | 1.096 |
| native | 2 | 1 | 1.001 | 0.984 | 1.005 | 1.080 |
| native | 2 | 2 | 1.034 | 1.189 | 0.962 | 1.098 |
| native | 2 | 3 | 1.078 | 1.080 | 0.932 | 1.090 |
| tika | 1 | 1 | 0.884 | 1.123 | 1.130 | 1.054 |
| tika | 1 | 2 | 0.924 | 1.202 | 1.083 | 1.073 |
| tika | 1 | 3 | 0.962 | 1.226 | 1.039 | 1.065 |
| tika | 2 | 1 | 0.948 | 1.106 | 1.040 | 1.063 |
| tika | 2 | 2 | 0.977 | 1.094 | 1.002 | 1.089 |
| tika | 2 | 3 | 0.981 | 1.098 | 1.001 | 1.050 |

Native concurrency 2 misses the 1.05 throughput threshold at both budgets and
increases p95. At concurrency 3 the two-CPU native throughput benefit also exceeds
the allowed p95 regression. Tika throughput is lower in every stratum. Every raw
pair, neutral result and regression remains in `paired.json`; no stratum is omitted.
The outcome supports keeping the experiment opt-in. Even a screen pass would not
justify a default change without representative deployment evidence.

CPU and throttling describe the recorded measurement interval after load/warmup.
The memory high-water mark includes startup and warmup; file-cache charges can
predate a cell. Twelve requests cannot resolve a stable p99. Driver allocation
counters do not measure child/native allocations or native internal copies.
These synthetic fixtures do not establish representative multilingual OCR quality.

`raw.tar.gz` preserves every raw file, directory and exact bridge-link target from
both layers, the host window, new admission packet, independent archive/source
verification and aggregate cell admission. Only transport owner IDs and timestamps
are normalized; contents, paths, types, modes and link targets remain recorded.
Small actual SDK and driver producer receipts are also included in `provenance`.
The packet is self-contained for statistical replay. Native archives, model weights
and executable bytes remain outside Git, identified by their immutable receipts;
this packet is not a source-relinking distribution or a signed release artifact.

Run `bash scripts/replay-continuous-pilot.sh PATH_TO_THIS_DIRECTORY NEW_OUTPUT`
from a checkout containing the recorded analyzer. This checks packet integrity,
extracts into a new private directory, revalidates all original manifests and
required outcome/resource/cleanup evidence, reruns the exact rational screen and
requires byte-identical `paired.json`. No OCR, Docker daemon, model load or network
access is used. Python is limited to this analysis tooling; the OCR runtime is Rust.
