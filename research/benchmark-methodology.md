# Benchmark evidence and acceptance criteria

Research date: 2026-10-17. Exa bookkeeping: 9 searches, `sources_reviewed: 45`
(the sum of requested result slots, not 45 independent validated sources), plus
8 directly fetched URLs. Primary sources below were inspected for the claims
used in the experiment design.

## What the CPU budget means

The application budget includes Tika's parent JVM, forked JVM, PDF rendering,
the Rust compatibility process, HTTP service, and every OCR worker/thread.
The load generator and measurement process run on a disjoint logical CPU.
There is no GPU or external OCR service in the measured path.

Linux cgroup controls are hierarchical: a child cannot override its ancestors'
restrictions. `cpu.max = 100000 100000` means one CPU's aggregate time per
100 ms period; `200000 100000` means two. Read `cpu.stat` before/after each
measurement and retain `usage_usec`, `nr_periods`, `nr_throttled`, and
`throttled_usec`. Do not add parent usage to child usage: parent usage already
includes descendants. Current kernel documentation also distinguishes local
throttling from throttling inherited from ancestors.
[Linux cgroup v2](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html)

Quota alone can allow a workload to run briefly on many cores. Pin every
application process/container to the same one- or two-CPU set when comparing
limited hardware. Identify SMT siblings, CPU model, frequency policy, RAM,
kernel, and background load. A cpuset restricts eligibility; it does not reserve
the selected CPUs against unrelated host work.
[Docker CPU constraints](https://docs.docker.com/engine/containers/resource_constraints/)

Compose `cpus` is a per-container limit. Giving two containers one CPU each
permits two CPUs in total unless a shared parent budget or common one-CPU
cpuset also constrains them. Compose exposes `cpuset` and `cgroup_parent`.
[Compose services](https://docs.docker.com/reference/compose-file/services/)

Kubernetes container limits add up to the Pod's effective budget. Pod-level
resources allow a shared budget where `PodLevelResources` is supported and
enabled (documented as beta since v1.34). A portable split such as 250m/750m
also caps the sum at one CPU, but can strand capacity when one pipeline stage
is idle; such a split requires tuning. CPU requests govern scheduling/weight;
requests alone are not a hard ceiling.
[Kubernetes resource management](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)

## Accuracy must survive the migration

Run two comparisons: identical encoded images to isolate the OCR service path,
and complete Tika ingestion with identical PDF/rendering/parser settings.
Test native-text PDFs separately in AUTO mode and require zero OCR calls;
include image-only and mixed PDFs in the deployment acceptance corpus.

Use held-out, manually transcribed production documents, grouped by source
document and stratified by language, scan quality, layout, tables, tiny text,
rotation, and blank pages. Keep template duplicates and variants in one split.
Never train/tune thresholds on the final acceptance split. The supplied
synthetic fixtures validate implementation and reproduce integration failures;
they cannot establish production-wide accuracy or a global best model.
OCR-D explicitly calls for representative images with manual ground truth and
notes that reading order affects text metrics.
[OCR-D evaluation](https://ocr-d.de/en/spec/ocrd_eval)

The included harness reports ordinary edit-distance CER and WER divided by
ground-truth units; insertions can produce values above 100%. It uses NFC
**Unicode codepoints**, not OCR-D's grapheme-cluster character definition.
Primary text scores collapse Unicode whitespace and preserve case,
punctuation, accents, spelling, numbers, and reading order. A second strict
score preserves whitespace after CRLF/CR normalization. Do not compare these
numbers with a differently normalized benchmark. OCR-D's normalized CER has a
different denominator and must not be presented as the same metric.
[OCR-D metric definitions](https://ocr-d.de/en/spec/ocrd_eval)

Score the first measured output once per unique case/configuration. Repeated
timing runs do not create independent accuracy samples. Report micro averages
(sum of edits / sum of reference units), per-slice results, blank-page false
positives, all failed pages, and output changes across repeats. Failures remain
failures and contribute empty predictions/all deletions on nonempty pages.

For production acceptance, compare candidate minus baseline scores with paired
document-cluster resampling. The harness supplies an explicitly labelled
95% percentile-bootstrap interval; a five-document synthetic corpus is much
too small for a production guarantee. SciPy documents paired resampling and
BCa intervals if a richer statistical analysis is required; its BCa default
must not be confused with the harness's percentile implementation.
[SciPy bootstrap](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.bootstrap.html)

## Performance must include the complete service

Measure startup/readiness separately from the first client request and warmed
requests. Readiness of this backend includes model verification, loading, and
an internal warmup. Process restarts do not imply a cold filesystem cache.
Downloads, image pulls, and provisioning are separate experiments.

The included runner measures bounded, closed-loop corpus makespan and
successful pages/second. Retain every latency/status and report p50, p95, p99,
maximum, failures, and sample size. Fewer than 1,000 successful requests is an
explicit warning for tail interpretation, not a claim that 1,000 automatically
makes a tail estimate reliable. Use multiple independent runs and a stable
representative corpus; do not claim a p99 SLO from a few dozen requests.

Production SLO acceptance additionally needs open-loop arrival-rate tests.
Closed-loop clients reduce arrival rate when the server slows, which can hide
queueing under overload. With k6, use `constant-arrival-rate` or
`ramping-arrival-rate`, provision sufficient VUs in advance, and count dropped
iterations, timeouts, 429/503 responses, and unbounded queue growth.
[k6 open/closed models](https://grafana.com/docs/k6/latest/using-k6/scenarios/concepts/open-vs-closed/)

Start Tesseract with `OMP_THREAD_LIMIT=1`, then compare two independent
single-thread requests against one two-thread request at a total two-CPU
budget. Tesseract's FAQ specifically warns that four threads can be slower
on a two-core machine. Model families (`tessdata`, `tessdata_fast`,
`tessdata_best`) must be named and hashed rather than hidden behind an engine
name.
[Tesseract FAQ](https://tesseract-ocr.github.io/tessdoc/FAQ.html)

ONNX Runtime's default intra-op thread choice follows available physical
cores, and spinning can spend CPU while waiting. Explicitly control thread
counts and compare one worker/two intra-op threads against two workers/one
thread. Extra workers and extra intra-op threads are not independent budgets.
Profile spinning changes as separate candidates; do not assume they improve
every workload.
[ONNX Runtime threading](https://onnxruntime.ai/docs/performance/tune-performance/threading.html)

Indra's batching here is width-bucketed **line-crop tensor batching within a
page**. Record actual tensor batch-size histograms and padding, not just the
configured maximum or concurrent HTTP requests. Compare batch 1 with the
configured maximum using the same model/settings and verify output fidelity.
Cross-request fusion, INT8, or optional math must receive their own accuracy
and latency measurements before any performance claim.

## Acceptance gate

Choose tolerances from the actual ingestion workload before the held-out run.
Require no silent page loss, native-text bypass, stable output ordering,
bounded queues, bounded memory, controlled overload responses, and measured
accuracy non-inferiority for every critical slice. Choose the fastest
configuration that satisfies those constraints. Report the measured tradeoff
instead of promising simultaneously maximal accuracy and throughput.

Build correctness and signed-image provenance belong in normal CI. Run
performance gates on reserved, identified self-hosted hardware, with engine
order balanced across repetitions and image/model/corpus/configuration hashes
in the report. A signature proves provenance; it does not prove OCR accuracy
or reproducible speed.
[GitHub self-hosted runners](https://docs.github.com/en/actions/concepts/runners/self-hosted-runners)
