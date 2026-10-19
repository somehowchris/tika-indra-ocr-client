# Actual perf tool admission: 2026-10-18

Owned tool-only probes passed at CPU0/one CPU and CPUs0,1/two CPUs with exact
`cpu-clock:u`,99Hz,strict frequency,DWARF16384,32MiB rings,256MiB output cap,
inherited child tasks and FIFO-controlled enable/disable. Each child ran a
one-second GNU timeout-bounded bash loop. No OCR model, OCR latency measurement,
Criterion experiment or production memory claim is part of this proof.

The immutable tools image is
`sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142`.
Its complete internal tool identity manifest passed. Actual perf6.12.107 records,
reports and scripts were read successfully. CPU1 captured99 samples;CPU2 captured98.
Both reports record zero lost samples. All owned containers stopped and were
removed with cleanup0; the memory cgroups remained3GiB with no swap orOOM events.
The successful controller sources, kernel/process limits, actual masks, ancestor
checks, full verbose mappings and raw textual report/script are retained here.

Both profiles opened96 event FDs and64 mapped rings:32 sample rings each33,558,528B
plus32 dummy/BPF metadata rings each528,384B, totaling1,090,781,184B of mapped
buffer capacity. The actual bounded memlock soft/hard allowance was1,140,850,688B
(1088MiB). The sample event remains restricted to the inherited target; metadata
opens include dummy events and are not sampling outside that target. No `-C`,
per-thread mode, host sysctl change or additional capability was used. Docker's
actual capability spelling was `CAP_PERFMON`; the guard admits exactly this set
or its `PERFMON` alias.

Cgroup memory peaks were10,117,120B and10,567,680B. Mapped kernel buffer capacity
and cgroup/RSS charges are different measurements; the mapping allowance belongs
to profiler instrumentation and does not describe OCR memory. Actual OCR captures
still require their original output/resource/sample-quality gates.

Failed v2/default-memlock and v3/64MiB attempts are retained. V4 repeated64MiB
with verbose mapping diagnostics and showed the32-host-CPU multiplicity even
with child affinity0. They failed255 before sampling and cleaned up successfully.
The first pre-Docker setup attempt failed1 because the219 private worktree does
not contain the later shared cgroup helper; its untouched source/output directory
remains `/vol/md1/tika-indra-artifacts/profile-perf-probe-6ac9c60-20261018`.
Later attempts sourced the already-integrated read-only helper explicitly.

Raw perf.data stays in the corresponding local artifact directories ending-v5
and-v6-cpu2. Its container-root ownership and0600 mode were preserved. The two
raw file hashes were obtained with a separate finite, owned, read-only,
capdropALL hash container; its successful cleanup receipt is retained. Full
raw attempts and that container's source/inspection remain under
`/vol/md1/tika-indra-artifacts/profile-perf-probe-hashes-20261018`.
