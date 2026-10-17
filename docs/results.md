# Results, evidence and deployment decision

Measured on **2026-10-17**. Indra is a promising replacement for this workload: it improved measured throughput, handled the two-column fixture well, and worked through the exact Tika 3.3.2 integration. The evidence supports a controlled migration trial. It does **not** establish a universal accuracy winner, a production tail-latency guarantee, or the speed of the supplied Docker CPU partitions.

Start with one worker, one inference thread, standard FP32 models, upstream line batching, Tika AUTO and 300-DPI grayscale rendering. Retain the Tesseract image as the comparison and rollback path. Choose a larger worker/thread or math profile only after a matched test on the documents that matter.

## What was actually proved

| Gate | Result and scope |
|---|---|
| Exact Tika 3.3.2 CLI, PDF configuration and error behavior | Exercised with the official checksum-verified server JAR; [contract evidence](../validation/tika-contract.json) |
| Rust build and protocol | Four unit tests, strict Clippy, GNU static ELF checks, ten real subprocess/HTTP protocol tests and combined health checks passed; [build](../validation/rust-checks.json), [protocol](../validation/bridge-protocol.json), [health](../validation/gnu-bridge-health.json) |
| Real Indra service | Twelve input/HTTP tests and two actual worker-death/replacement-failure tests passed with locked models; [results and logs](../validation/backend-real/tests.json) |
| Whole-stack native performance | Matched one-CPU and two-CPU process-tree affinity experiments, actual OCR, retained request outputs, three timing repetitions and no observed affinity escape in accepted primary runs |
| Exact runtime filesystem compatibility | Pinned Java 21 and Python 3.13 distroless filesystems executed with the real bridge/models, non-root IDs, read-only roots and bounded noexec tmpfs; Indra and Tesseract stacks both passed image, scanned-PDF and native-PDF checks at one and two eligible CPUs; [rootfs evidence](../validation/rootfs/) |
| Docker application execution / CFS enforcement | **Blocked here:** the isolated Docker daemon could not create application container cgroups because `/sys/fs/cgroup` is read-only. Rootfs execution does not establish Docker or Compose execution. The failure is retained with the rootfs evidence. |
| Kubernetes deployment and admitted resource limits | Manifests authored and syntax checked; no cluster was available for execution |
| GitHub Actions, published signatures and SBOM attestations | Workflows authored and source-reviewed; no workflow run, registry publication or successful signing operation is claimed |
| Production accuracy, long documents, overload, p99 and memory-limit survival | Not established by the small fixture suite; concrete acceptance measurements appear below |

The retained benchmark package contains **786 successful accepted timed requests across 13 completed experiments**, including the native-PDF bypass controls. It records zero failed accepted requests and no observed affinity escapes in those runs. Invalid or blocked attempts remain separately identified. The 1,200-call bridge microbenchmark and correctness/fault checks are additional, distinct evidence.

## Hardware and fairness

The guest identifies an **Intel Xeon Platinum 8370C at 2.80 GHz**, x86-64. It exposes nine logical CPUs, an eight-CPU host cgroup quota and 8 GiB of host cgroup memory. The OCR experiment uses **CPU 2 only** or **CPUs 2 and 3 together** for the complete application. The load generator and process monitor use CPU 8. The guest reports distinct core IDs for CPUs 2 and 3, with no exposed sibling; this does not prove exclusive physical cores on the host.

Every observed Tika parent/child JVM, Rust OCR subprocess, Python supervisor and Indra worker/thread must stay inside the selected application CPU set. This bounds simultaneous execution to one or two logical CPUs. It is **inherited Linux affinity**, not an isolated one/two-CPU CFS quota. Other validation work used different CPU sets, but the shared VM, cache, storage and memory bandwidth can still vary. Per-repetition timings expose substantial variance.

The raw comparison uses the same encoded image bytes. Tesseract starts a CLI process per request; Indra keeps models resident behind HTTP. Those are the actual integration paths, so the result includes the benefit of model residency and is not a pure neural-kernel comparison. The Tika comparison uses the same JAR, input documents, grayscale 300-DPI rendering, OCR-only policy and JVM settings for both engines. Native-text AUTO is measured separately.

Native timing uses Python **3.13.16**, Java **17.0.20** and Tesseract **5.3.4**. The verified final-image filesystems use Python **3.13.5**, Java **21.0.12.1+1** and Tesseract **5.5.0**. Native Tika has a 256-MiB parent heap and 512-MiB parser heap; deployment defaults use 128/512 MiB. The native two-CPU throughput experiment uses two one-thread workers and two concurrent requests. Image-root smoke checks establish compatibility of final runtimes; they do not substitute for a performance rerun of those versions and resource policies.

The image-root smoke tests used one Indra worker/one thread at both affinity masks. The recommended two-worker deployment setting comes from the native throughput experiment. The GNU bridge in the smoke check is the native-built binary whose hash is recorded; no Docker-produced bridge digest is claimed. Source/relink packaging was added after the inference checks and is a separate build gate. None of these checks is a byte-for-byte execution of a completed Docker build.

## Main performance results

Throughput is successful pages divided by the measured request-batch makespan, excluding startup, the designated first client warmup and offline accuracy scoring. Latency includes the complete client request path. One-CPU runs use concurrency one; two-CPU throughput runs use concurrency two. A higher concurrency can raise throughput and individual request latency together.

### Whole Tika stack: distribution Tesseract model

Each row covers **15 unique cases × 3 timing repetitions = 45 successful requests**, with zero request failures. The corpus contains twelve images, two scanned PDFs and one native PDF deliberately forced through OCR for this comparison.

| Total eligible CPUs | Engine | Pages/s | p50 ms | p95 ms | CER | WER |
|---:|---|---:|---:|---:|---:|---:|
| 1 | Tika + Tesseract distribution English | 0.917 | 993.3 | 2,407.4 | 2.1739% | 4.5190% |
| 1 | Tika + Indra | **1.705** | **385.3** | **1,586.5** | **0.1233%** | **0.8017%** |
| 2 | Tika + Tesseract distribution English | 1.058 | 1,653.3 | 3,697.6 | 2.1739% | 4.5190% |
| 2 | Tika + Indra | **1.895** | **801.8** | **2,233.4** | **0.1233%** | **0.8017%** |

These matched throughput point estimates favor Indra by **1.86× at one CPU** and **1.79× at two CPUs**. Indra's one-to-two-CPU increase is about **11%** in this series, while individual latency rises with concurrency. The distribution model is much faster than `tessdata_best`, so these are the more useful starting figures for an existing default-Tesseract installation. Its English model is locked by SHA-256 `7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2` in the experiment metadata.

Sources: [one CPU](../benchmarks/results/tika-stock-1/summary.json), [two CPUs](../benchmarks/results/tika-stock-2/summary.json), [all requests and per-run rates](../research/benchmark-results.md).

### Whole Tika stack: `tessdata_best` model

This earlier paired series uses the same 15-case design and 45 successful requests per row. It compares the slower best-model baseline with its own contemporaneous Indra runs; rates from the two series are not pooled into a single speedup.

| Total eligible CPUs | Engine | Pages/s | p50 ms | p95 ms | CER | WER |
|---:|---|---:|---:|---:|---:|---:|
| 1 | Tika + Tesseract `tessdata_best` | 0.402 | 2,354.6 | 4,501.0 | 2.1851% | 4.4461% |
| 1 | Tika + Indra | **1.503** | **443.4** | **1,873.4** | **0.1233%** | **0.8017%** |
| 2 | Tika + Tesseract `tessdata_best` | 0.376 | 4,302.6 | 10,990.5 | 2.1851% | 4.4461% |
| 2 | Tika + Indra | **1.790** | **812.7** | **2,455.4** | **0.1233%** | **0.8017%** |

These are approximately **3.74×** and **4.76×** throughput point estimates against the best-model baseline. Indra's own one-to-two-CPU increase is only about **19%** here. Tesseract's two-CPU aggregate is lower than its one-CPU result; one request took approximately 27.9 seconds. The three Tesseract two-CPU repetition rates range from 0.286 to 0.485 pages/s, making any general scaling claim inappropriate.

In both series, p95 values are descriptive order statistics from 45 requests. They are too small a sample for a production tail-latency promise. Raw JSON also reports p99 with an explicit warning. Between-series variation in Indra's rates is another reason to retain paired comparisons and avoid extrapolating these shared-host point estimates.

Sources: [one CPU](../benchmarks/results/tika-1/summary.json), [two CPUs](../benchmarks/results/tika-2/summary.json), [all requests and per-run rates](../research/benchmark-results.md).

### Direct image paths

Each row covers twelve unique images, 36 successful timed requests and zero failures.

| Total eligible CPUs | Engine | Pages/s | p50 ms | p95 ms | CER |
|---:|---|---:|---:|---:|---:|
| 1 | Tesseract distro English | 0.988 | 898.6 | 2,016.9 | 2.8542% |
| 1 | Tesseract best English | 0.509 | 2,249.1 | 3,753.8 | 2.8691% |
| 1 | Indra | **2.116** | **430.2** | **925.6** | **0.1195%** |
| 2 | Tesseract distro English | 1.394 | 1,533.5 | 2,291.2 | 2.8542% |
| 2 | Tesseract best English | 0.552 | 3,944.9 | 6,703.1 | 2.8691% |
| 2 | Indra | **2.968** | **681.2** | **925.3** | **0.1195%** |

Sources: [one CPU](../benchmarks/results/raw-1/summary.json), [accepted two-CPU run](../benchmarks/results/raw-2-verified/summary.json). The initial two-CPU attempt was rejected because process sampling caught `taskset` before it applied the workload affinity. It is preserved as an invalid attempt and excluded from these tables; the harness now waits for the actual workload mask before attaching its sampler.

## Accuracy: the near-tie hidden by the aggregate

The synthetic set has only **five source-document clusters**. Multiple images and PDFs are renditions of the same short operations report. Repeating those inputs three times increases timing observations, not independent accuracy evidence.

The headline error gap is concentrated in the two-column reading-order case and a multilingual Latin challenge. The baseline explicitly uses **English** trained data. Reading order is a real extraction concern, but this is not a comparison of every engine's best language-specific configuration.

After excluding the two challenge categories and the blank page:

| Matched ordinary-text subset | Reference codepoints | Tesseract distro errors | Tesseract best errors | Indra errors |
|---|---:|---:|---:|---:|
| Raw image paths | 6,101 | 9 (0.1475%) | 8 (0.1311%) | 7 (0.1147%) |
| Tika OCR-only paths | 8,333 | 12 (0.1440%) | 11 (0.1320%) | 10 (0.1200%) |

**Indra differs from `tessdata_best` by one character, and from the distribution model by two characters, on this ordinary-text subset.** This is useful evidence of output stability on those fixtures, not a broad accuracy superiority claim. The cluster bootstrap intervals in `summary.json` are exploratory because five clusters cannot represent production diversity.

CER is character edit distance divided by reference Unicode codepoints after NFC normalization and whitespace collapse; WER uses reference words. Case, punctuation and reading order remain significant. These are codepoints, not grapheme clusters. Strict whitespace-sensitive scores are also retained. Tika can emit formatting whitespace for a blank page: the normalized blank false-positive count is zero while strict whitespace output is nonempty. Normalization is explicit and must match the consuming application's needs.

### Public legacy scans, reported separately

Three pinned upstream Tesseract regression scans provide a second, non-synthetic diagnostic. Two include historical UNLV transcription markers and reading-order conventions. Their references have not been manually adjudicated into production-quality ground truth. Call these **legacy transcript disagreement**, not a clean estimate of production OCR accuracy.

| Engine | 1-CPU pages/s | 2-CPU pages/s | CER | WER |
|---|---:|---:|---:|---:|
| Tesseract distro English | 0.287 | 0.447 | 8.2484% | 13.7378% |
| Tesseract best English | 0.118 | 0.165 | 8.1999% | 13.2497% |
| Indra | **0.786** | **1.166** | **5.6890%** | **12.5523%** |

There were nine successful timed requests per engine at each budget, with no failures. Accuracy was unchanged across CPU budgets. The smaller WER difference is worth keeping alongside CER. The archive includes numeric observations, source hashes and retrieval code; it omits the scanned articles, their transcripts and predicted article text. [Results](../research/benchmark-results.md#public-regression-diagnostics), [immutable corpus metadata](../benchmarks/public-regression.lock.json).

## What the optimization experiments say

### Dynamic line batching

The backend uses upstream batching within each page. It sorts line crops by width, separates recognition scripts, and limits count and a width-based estimate of padded work. The estimate precedes rounding tensor width to a multiple of eight; it is not an exact final tensor-area limit. There is no cross-request tensor fusion or artificial batching delay. The [pinned preprocessing code](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/rec_preproc.py) and [pipeline](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py) are the authority for this behavior.

On six matched synthetic fixtures, three repetitions each:

| One-CPU setting | Pages/s | CER | Observed tensor batches |
|---|---:|---:|---|
| Recognition batch ceiling 1 | 2.802 | 0.1346% | B1 only |
| Recognition batch ceiling 64 | 2.986 | 0.1346% | B1–B9 |

The larger ceiling improved the aggregate point estimate by **6.55%**, with identical text. Per-run rates overlap substantially: 2.286–3.200 versus 2.917–3.095 pages/s. This does not prove a repeatable 6.55% improvement on another workload.

Actual shape instrumentation observed B1–B9 on the primary synthetic raw run, B1–B10 through Tika, and real **B64** calls on the public scans. Padding accounted for approximately 8.57%, 8.62% and 11.80% of the corresponding observed recognition/script tensors. These totals include warmups. In the batch ablation, padding rose from 0.81% with single-line batches to 8.82% with grouped lines: batching trades more padded work for fewer runtime calls. A configured ceiling of 64 is not proof that every request uses that size.

### Math and thread counts

The ordinary-Latin math toggle measured **2.784 pages/s off** and **2.581 pages/s on**, approximately 7.3% lower throughput with unchanged text score. This measures overhead on ordinary text, not equation accuracy. Formula transcription and formula-to-LaTeX correctness require a separate annotated corpus; enabling extra models everywhere is not an established highest-accuracy setting.

One-versus-two-thread controls use the same six cases and single request concurrency at a two-CPU affinity budget, with 18 successful requests per setting:

| Matched single-request setting | Pages/s |
|---|---:|
| Tesseract best, OpenMP limit 1 | 0.577 |
| Tesseract best, OpenMP limit 2 | 0.531 |
| Indra, one worker / one intra-op thread | **2.560** |
| Indra, one worker / two intra-op threads | 1.825 |

More threads were slower here; text was unchanged within each engine. The supplied two-CPU throughput profile therefore uses two one-thread workers, matching the primary concurrent-throughput experiment, with its additional memory cost. This does not prove that profile wins at every arrival rate or under the deployment's hard quota partition. ORT session threads, worker processes and queue capacity are separate choices.

A separate rootfs-check listener occupied the supplemental experiment's initial port before two Indra cohorts could start. The runner refused to measure the unrelated server. The blocked attempts remain recorded; only those missing cohorts were completed on an unoccupied port. See [the matched thread report and recovery record](../research/benchmark-results.md). The performance controller and competing listener never shared the experiment CPU set.

### Rust release versus fat LTO

The bridge is dependency-free Rust with concrete types and explicit bounded buffers. The production profile uses optimization level 3, thin LTO, one codegen unit and abort-on-panic. The `maxperf` profile changes LTO to fat and is a candidate, not a promise implied by its name.

The GNU release binary measured **1,228,480 bytes**; fat LTO measured **1,199,552 bytes**. Both had no ELF interpreter and no dynamic-library dependency entries. A dedicated microbenchmark ran **1,200 measured invocations**, plus 120 excluded warmups, with 256-KiB image uploads and 16-KiB validated responses. Its HTTP peer was an explicitly labeled stand-in: **no OCR occurred**.

| Application CPUs | Thin-LTO mean | Fat-LTO mean | Fat minus thin, paired 95% interval |
|---:|---:|---:|---:|
| 1 | 2.760 ms | 2.792 ms | +0.032 ms, interval −0.407 to +0.471 ms |
| 2 | 2.331 ms | 2.214 ms | −0.117 ms, interval −0.367 to +0.133 ms |

Three interleaved repetitions do not show a reliable fat-LTO improvement. Keep thin LTO. Bridge work is measured in milliseconds while OCR takes hundreds or thousands of milliseconds in these tests. No PGO, CPU-specific instruction target, allocator replacement or speculative Rust inference rewrite is justified by this result. [Summary](../validation/bridge-profile-summary.json), [all samples](../validation/bridge-profile-samples.json), [reproduction script](../scripts/measure_bridge.py).

## Memory, CPU time and startup

These are sampled whole-process-tree observations from the full Tika comparisons. The stock-model paired series records:

| Application CPUs | Engine | Peak summed RSS across repetitions | Startup/readiness range | Observed cohort CPU time |
|---:|---|---:|---:|---:|
| 1 | Tika + distribution model | 354–392 MiB | 5.61–6.94 s | 23.37–23.71 CPU-s |
| 1 | Tika + Indra | 549–600 MiB | 6.79–8.29 s | 17.41–18.53 CPU-s |
| 2 | Tika + distribution model | 420–444 MiB | 4.12–4.64 s | 32.42–40.06 CPU-s |
| 2 | Tika + Indra | 759–783 MiB | 5.39–7.24 s | 24.40–28.59 CPU-s |

The earlier `tessdata_best` paired series records:

| Application CPUs | Engine | Peak summed RSS across repetitions | Startup/readiness range | Observed cohort CPU time |
|---:|---|---:|---:|---:|
| 1 | Tika + best | 370–404 MiB | 6.03–9.35 s | 44.35–52.70 CPU-s |
| 1 | Tika + Indra | 545–602 MiB | 6.73–7.09 s | 17.88–19.83 CPU-s |
| 2 | Tika + best | 437–467 MiB | 4.46–5.52 s | 74.30–96.75 CPU-s |
| 2 | Tika + Indra | 748–784 MiB | 6.69–7.42 s | 28.62–31.60 CPU-s |

Summed RSS double-counts shared mappings and can miss short peaks. Sampled CPU is a lower bound when short-lived children disappear between samples. The cohort covers startup, internal model warmup, the first client warmup, measured requests and shutdown. Dividing that CPU total by measured pages would not produce a valid warm-only CPU/page metric. These runs had the host's 8-GiB memory boundary, **not** an isolated 3-GiB application cap.

The backend's ready state includes verified models and an internal warmup. Readiness timing is cold-process startup with potentially warm filesystem cache; no cold-boot, cold-download or cleared-page-cache claim is made. First client request latencies are retained separately. Native PDF AUTO controls at both CPU budgets made zero Indra OCR requests, including warmups; their three timings per engine per budget are correctness controls, not useful speed comparisons.

## Measurements still needed for acceptance

The deployment's CPU limits sum to one or two, with 3 GiB total memory. Its fixed Tika/Indra quota partitions cannot borrow each other's idle budget. Benchmark those exact signed image digests on a Docker or Kubernetes host before carrying over the native rates. Use the supplied [signed-image wrapper](../benchmarks/docker_run.py) for a shared-CPU-set comparison, and separately retain the default partitions for a deployment-specific comparison.

| Question | Concrete next experiment and recorded outcome |
|---|---|
| Is accuracy acceptable for the actual documents? | Separate tuning and held-out sets, independent document clusters, adjudicated reading order, per-language CER/WER, field exact match for IDs/amounts, and error inspection. Configure equivalent Tesseract languages. Retain originals, truth and hashes under the appropriate rights. |
| Is 300 DPI the right rendering choice? | Render the same source PDFs at 200/300/400 DPI in both engines; report render time, raster pixels, OCR time, field accuracy and memory. Existing source-image variants do not replace this controlled PDF experiment. |
| Does math help? | Annotated formulas, exact/normalized LaTeX or expression equivalence, text regressions, table structure, latency and memory with math off/on. |
| Which two-CPU deployment wins? | One worker/one thread, one/two threads, and two/one threads, matched caller concurrency; default per-container quotas versus a genuine shared total budget. Record admitted resources and leaf/parent cgroup counters without double-counting. |
| Is the latency target sustainable? | Open-loop offered-load sweep, queued and rejected requests, completions, drops, timeouts and at least thousands of requests per setting over several independent runs. Keep load generation outside the application budget. |
| Does it survive its memory limit? | Long PDFs, large pages, maximum permitted TIFF frames, concurrency, cancellations and native worker failures under the actual 3-GiB cap; record `memory.peak`, OOM events, restart reason and recovery time. |
| Is the release chain real? | Run CI, smoke the exact published digests at both budgets, verify signatures/provenance/SBOM, then deploy those digests and repeat the workload. Workflow presence alone is not evidence of execution. |

This investigation evaluated practical integration choices, model availability, line batching, worker/thread strategies, math overhead, two Rust profiles, output/error handling, runtime ABI compatibility and two constrained native CPU budgets. Cross-page batching, quantization, alternative execution providers, handwriting, other-script accuracy, a Rust inference port and deployment tail latency remain unmeasured. They should remain explicit experiments rather than advertised features.
