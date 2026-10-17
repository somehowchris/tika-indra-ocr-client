# Measured OCR results

Measured locally on 2026-10-17 using native processes and the verified static GNU/glibc bridge.
These are small reproducible experiments, not production accuracy or p99 SLO guarantees.
All application processes shared CPU 2 for the one-CPU runs or CPUs 2 and 3 for the two-CPU runs;
the load generator/monitor used CPU 8. No GPU, external OCR API, Docker, or Kubernetes runtime was used.

The guest exposes CPUs 2 and 3 as different core IDs, with no exposed sibling on either logical CPU.
This virtualized/shared environment still does not establish exclusive physical cores or portable scaling.
Two-CPU throughput runs use concurrency 2; one-CPU runs use concurrency 1. Compare latency in that context.

Actual batch-shape instrumentation was enabled, and its unisolated overhead is included in Indra timings.
The frozen measured backend is archived; later production recovery/health checks are separate validation.

## Primary synthetic comparisons

Raw service comparisons use 12 images and five source-document clusters. Tika OCR-only uses those
12 images plus three PDFs, 15 cases total. Several renditions reuse one source text. Accuracy is scored
once per case; three timing repetitions do not increase the number of independent documents.
`Stock`/`distro` denotes this host's Tesseract 5.3.4 and hashed installed English model. The separately
pinned `tessdata_best` model is a different baseline. Native Tika used the recorded Java 17 executable.
Use each paired stock-model run to discuss replacement of that baseline; label best-model ratios explicitly.
The large error-rate difference is concentrated in the two-column and multilingual challenge cases;
the explicitly configured Tesseract language is `eng`. It is not a claim of a global language-agnostic optimum.

CER/WER below use NFC codepoints and collapsed whitespace while retaining case/punctuation/reading order.
Strict whitespace scores are in the JSON evidence. Compare accuracy only within a matched corpus.

A post hoc sensitivity split excluding the multilingual challenge, two-column reading-order case, and blank
shows that ordinary-text accuracy was close. This is a diagnostic split, not a revised acceptance corpus:

| Path | Reference codepoints | Distro Tesseract errors | Tesseract best errors | Indra errors |
| --- | ---: | ---: | ---: | ---: |
| Raw images | 6101 | 9 | 8 | 7 |
| Tika OCR-only, best baseline | 8333 | — | 11 | 10 |
| Tika OCR-only, distro baseline | 8333 | 12 | — | 10 |

These slice scores are identical across the valid one/two-CPU runs. The full-corpus scores remain below.

### raw-1: Synthetic images

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tesseract-distro | 12 | 36 | 0 | 2.8542 | 5.7282 | 0.988 | 898.6 |
| tesseract-best | 12 | 36 | 0 | 2.8691 | 5.6311 | 0.509 | 2249.1 |
| indra | 12 | 36 | 0 | 0.1195 | 0.7767 | 2.116 | 430.2 |

[Summary](../benchmarks/results/raw-1/summary.json) · [Every request](../benchmarks/results/raw-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/raw-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tesseract-distro | 0.687, 1.302, 1.232 | 57.9, 57.9, 57.8 | 17.33, 9.60, 10.12 | 0.0, 0.0, 0.0 |
| tesseract-best | 0.502, 0.499, 0.528 | 67.4, 67.4, 67.4 | 23.69, 27.15, 24.13 | 0.0, 0.0, 0.0 |
| indra | 1.483, 2.935, 2.484 | 245.4, 258.1, 243.8 | 10.70, 5.51, 6.24 | 3174.6, 1126.0, 1012.2 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra: B1–B9, 8.57% padding.

### raw-2-verified: Synthetic images

Total application budget: **2 logical CPU(s)**; request concurrency: **2**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tesseract-distro | 12 | 36 | 0 | 2.8542 | 5.7282 | 1.394 | 1533.5 |
| tesseract-best | 12 | 36 | 0 | 2.8691 | 5.6311 | 0.552 | 3944.9 |
| indra | 12 | 36 | 0 | 0.1195 | 0.7767 | 2.968 | 681.2 |

[Summary](../benchmarks/results/raw-2-verified/summary.json) · [Every request](../benchmarks/results/raw-2-verified/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/raw-2-verified/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tesseract-distro | 1.299, 1.393, 1.505 | 99.7, 99.7, 99.9 | 18.23, 17.26, 15.90 | 0.0, 0.0, 0.0 |
| tesseract-best | 0.524, 0.556, 0.578 | 120.5, 119.6, 119.0 | 45.09, 44.31, 42.93 | 0.0, 0.0, 0.0 |
| indra | 2.930, 2.862, 3.123 | 429.3, 426.2, 437.1 | 10.15, 10.27, 9.75 | 1918.6, 1749.5, 1750.5 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra: B1–B9, 8.57% padding.

### tika-stock-1: Tika OCR-only, distro baseline, 300 DPI

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-stock | 15 | 45 | 0 | 2.1739 | 4.5190 | 0.917 | 993.3 |
| tika-indra | 15 | 45 | 0 | 0.1233 | 0.8017 | 1.705 | 385.3 |

[Summary](../benchmarks/results/tika-stock-1/summary.json) · [Every request](../benchmarks/results/tika-stock-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/tika-stock-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-stock | 0.845, 1.030, 0.895 | 392.3, 377.7, 354.2 | 23.71, 23.37, 23.63 | 6459.5, 6940.3, 5606.2 |
| tika-indra | 1.614, 1.621, 1.912 | 599.9, 548.8, 560.1 | 17.66, 18.53, 17.41 | 8294.4, 6792.2, 7484.7 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: tika-indra: B1–B10, 8.62% padding.

### tika-stock-2: Tika OCR-only, distro baseline, 300 DPI

Total application budget: **2 logical CPU(s)**; request concurrency: **2**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-stock | 15 | 45 | 0 | 2.1739 | 4.5190 | 1.058 | 1653.3 |
| tika-indra | 15 | 45 | 0 | 0.1233 | 0.8017 | 1.895 | 801.8 |

[Summary](../benchmarks/results/tika-stock-2/summary.json) · [Every request](../benchmarks/results/tika-stock-2/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/tika-stock-2/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-stock | 1.152, 1.033, 1.001 | 443.8, 419.9, 429.7 | 32.42, 39.41, 40.06 | 4115.0, 4643.8, 4536.3 |
| tika-indra | 1.880, 1.791, 2.029 | 774.2, 758.9, 782.7 | 24.40, 28.20, 28.59 | 5389.3, 5837.1, 7241.9 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: tika-indra: B1–B10, 8.62% padding.

### tika-1: Tika OCR-only, tessdata_best baseline, 300 DPI

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-best | 15 | 45 | 0 | 2.1851 | 4.4461 | 0.402 | 2354.6 |
| tika-indra | 15 | 45 | 0 | 0.1233 | 0.8017 | 1.503 | 443.4 |

[Summary](../benchmarks/results/tika-1/summary.json) · [Every request](../benchmarks/results/tika-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/tika-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-best | 0.349, 0.441, 0.429 | 404.0, 375.3, 370.2 | 52.70, 45.08, 44.35 | 9348.4, 6697.0, 6027.4 |
| tika-indra | 1.352, 1.585, 1.599 | 602.4, 545.4, 558.9 | 18.50, 19.83, 17.88 | 7090.3, 6744.5, 6727.4 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: tika-indra: B1–B10, 8.62% padding.

### tika-2: Tika OCR-only, tessdata_best baseline, 300 DPI

Total application budget: **2 logical CPU(s)**; request concurrency: **2**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-best | 15 | 45 | 0 | 2.1851 | 4.4461 | 0.376 | 4302.6 |
| tika-indra | 15 | 45 | 0 | 0.1233 | 0.8017 | 1.790 | 812.7 |

[Summary](../benchmarks/results/tika-2/summary.json) · [Every request](../benchmarks/results/tika-2/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/tika-2/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-best | 0.286, 0.415, 0.485 | 466.8, 437.0, 456.2 | 96.75, 83.04, 74.30 | 4459.4, 5520.2, 4906.7 |
| tika-indra | 1.672, 1.801, 1.913 | 783.7, 765.1, 747.6 | 28.62, 31.60, 29.59 | 6690.9, 7421.5, 7109.9 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: tika-indra: B1–B10, 8.62% padding.

## Controls and targeted ablations

### native-1: Native PDF AUTO control

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-best | 1 | 3 | 0 | 0.0000 | 0.0000 | 11.818 | 81.5 |
| tika-indra | 1 | 3 | 0 | 0.0000 | 0.0000 | 15.965 | 66.4 |

[Summary](../benchmarks/results/native-1/summary.json) · [Every request](../benchmarks/results/native-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/native-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-best | 14.483, 12.200, 9.724 | 236.7, 300.7, 236.1 | 6.86, 7.88, 20.23 | 6036.5, 6740.9, 20733.8 |
| tika-indra | 19.334, 14.947, 14.433 | 407.5, 407.7, 406.1 | 7.60, 10.21, 8.10 | 6715.6, 9178.0, 7266.3 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed backend OCR requests including warmups: **0**. This is a bypass correctness control,
not a useful throughput benchmark with only one distinct document.

### native-2: Native PDF AUTO control

Total application budget: **2 logical CPU(s)**; request concurrency: **2**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tika-tesseract-best | 1 | 3 | 0 | 0.0000 | 0.0000 | 24.677 | 41.8 |
| tika-indra | 1 | 3 | 0 | 0.0000 | 0.0000 | 23.164 | 41.2 |

[Summary](../benchmarks/results/native-2/summary.json) · [Every request](../benchmarks/results/native-2/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/native-2/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tika-tesseract-best | 22.549, 29.978, 22.798 | 234.5, 236.1, 233.7 | 9.20, 9.27, 7.89 | 4713.8, 4553.1, 3911.0 |
| tika-indra | 25.962, 23.881, 20.360 | 537.4, 540.4, 539.9 | 11.48, 9.91, 27.84 | 6830.8, 5338.1, 20979.2 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed backend OCR requests including warmups: **0**. This is a bypass correctness control,
not a useful throughput benchmark with only one distinct document.

### batch-1: Matched batch-size ablation

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| indra-batch1 | 6 | 18 | 0 | 0.1346 | 0.8696 | 2.802 | 335.6 |
| indra-batch64 | 6 | 18 | 0 | 0.1346 | 0.8696 | 2.986 | 323.4 |

[Summary](../benchmarks/results/batch-1/summary.json) · [Every request](../benchmarks/results/batch-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/batch-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| indra-batch1 | 2.286, 3.200, 3.118 | 235.1, 240.6, 236.8 | 3.70, 2.94, 2.83 | 809.6, 710.3, 819.0 |
| indra-batch64 | 2.917, 3.095, 2.951 | 239.8, 243.4, 230.7 | 3.21, 2.98, 3.05 | 813.4, 948.3, 813.2 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra-batch1: B1–B1, 0.81% padding; indra-batch64: B1–B9, 8.82% padding.

Actual tensor batch-size histograms, including warmups:

```json
{
  "indra-batch1": {
    "rec:B1": 287,
    "script:B1": 287
  },
  "indra-batch64": {
    "rec:B1": 15,
    "rec:B2": 22,
    "rec:B3": 8,
    "rec:B9": 7,
    "script:B1": 15,
    "script:B2": 22,
    "script:B3": 8,
    "script:B9": 7,
    "rec:B7": 4,
    "script:B7": 4,
    "rec:B5": 7,
    "rec:B6": 7,
    "script:B5": 7,
    "script:B6": 7,
    "rec:B8": 3,
    "script:B8": 3,
    "rec:B4": 3,
    "script:B4": 3
  }
}
```

The configured ceiling is 64 lines. It does not mean any request formed a 64-line tensor.

### threads-2-completed: Matched thread-count ablation

Total application budget: **2 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tesseract-best-omp1 | 6 | 18 | 0 | 6.2921 | 11.5217 | 0.577 | 1864.5 |
| tesseract-best-omp2 | 6 | 18 | 0 | 6.2921 | 11.5217 | 0.531 | 1704.4 |
| indra-threads1 | 6 | 18 | 0 | 0.1346 | 0.8696 | 2.560 | 379.6 |
| indra-threads2 | 6 | 18 | 0 | 0.1346 | 0.8696 | 1.825 | 553.3 |

[Summary](../benchmarks/results/threads-2-completed/summary.json) · [Every request](../benchmarks/results/threads-2-completed/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/threads-2-completed/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tesseract-best-omp1 | 0.548, 0.608, 0.577 | 67.4, 67.4, 67.4 | 12.90, 10.34, 11.01 | 0.0, 0.0, 0.0 |
| tesseract-best-omp2 | 0.588, 0.609, 0.433 | 67.4, 67.4, 67.4 | 20.74, 17.35, 24.15 | 0.0, 0.0, 0.0 |
| indra-threads1 | 2.538, 2.556, 2.587 | 244.4, 242.8, 245.7 | 3.91, 3.63, 4.07 | 1031.4, 922.2, 1462.6 |
| indra-threads2 | 1.790, 1.761, 1.932 | 249.8, 236.7, 248.5 | 10.30, 9.55, 8.70 | 1343.2, 1117.2, 1116.8 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra-threads1: B1–B9, 8.82% padding; indra-threads2: B1–B9, 8.82% padding.

Two Indra cohorts were blocked before launch by an unrelated server on the selected port.
The harness refused to use that server. Only those blocked cohorts were completed on a new loopback port;
the original 12 blocked attempts and all replacement observations remain under `attempts/`, with
[explicit recovery provenance](../benchmarks/results/threads-2-completed/recovery.json).

At concurrency 1, one intra-op thread was faster here than two. The practical two-CPU throughput
default remains two workers with one intra-op thread each, subject to the higher resident-memory cost.
That judgment combines the C1 thread ablation with the separate C2 primary run; it is not a matched
proof that one worker/two threads loses at every arrival rate or on other CPUs.

### math-1: Math toggle on ordinary Latin text

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| indra-math-on | 6 | 18 | 0 | 0.1346 | 0.8696 | 2.581 | 373.0 |
| indra-math-off | 6 | 18 | 0 | 0.1346 | 0.8696 | 2.784 | 346.9 |

[Summary](../benchmarks/results/math-1/summary.json) · [Every request](../benchmarks/results/math-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/math-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| indra-math-on | 2.696, 2.349, 2.734 | 262.9, 255.1, 249.0 | 3.71, 3.89, 3.67 | 1137.2, 1134.1, 1350.0 |
| indra-math-off | 2.829, 2.839, 2.689 | 239.7, 249.8, 239.4 | 3.35, 3.13, 3.41 | 919.2, 824.0, 936.3 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra-math-on: B1–B9, 8.82% padding; indra-math-off: B1–B9, 8.82% padding.

This toggle was tested on ordinary Latin text. It measures overhead/output stability there;
it does not establish equation recognition accuracy or the best math profile.

## Public regression diagnostics

Reported separately. Three pinned Tesseract regression scans include legacy UNLV
transcription markers and historical reading-order conventions. These references are not
adjudicated modern production ground truth. The source corpus was not used to tune models.
Only numeric results, hashes, manifest, and retrieval code are redistributed; no scan,
source article transcription, or predicted article text is included.

### public-1: Public legacy regression diagnostics

Total application budget: **1 logical CPU(s)**; request concurrency: **1**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tesseract-distro | 3 | 9 | 0 | 8.2484 | 13.7378 | 0.287 | 3968.7 |
| tesseract-best | 3 | 9 | 0 | 8.1999 | 13.2497 | 0.118 | 11753.0 |
| indra | 3 | 9 | 0 | 5.6890 | 12.5523 | 0.786 | 1489.5 |

[Summary](../benchmarks/results/public-1/summary.json) · [Every request](../benchmarks/results/public-1/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/public-1/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tesseract-distro | 0.222, 0.374, 0.306 | 43.0, 43.0, 43.0 | 20.41, 12.12, 9.80 | 0.0, 0.0, 0.0 |
| tesseract-best | 0.132, 0.113, 0.112 | 53.2, 53.2, 53.2 | 36.93, 36.71, 27.53 | 0.0, 0.0, 0.0 |
| indra | 0.605, 1.002, 0.858 | 327.2, 335.6, 310.8 | 7.46, 5.99, 5.10 | 1030.1, 1216.3, 1241.7 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra: B1–B64, 11.80% padding.

### public-2: Public legacy regression diagnostics

Total application budget: **2 logical CPU(s)**; request concurrency: **2**.
Affinity validation: **passed**.

| Engine | Unique cases | Successful requests | Failures | CER % | WER % | Pages/s | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| tesseract-distro | 3 | 9 | 0 | 8.2484 | 13.7378 | 0.447 | 6124.3 |
| tesseract-best | 3 | 9 | 0 | 8.1999 | 13.2497 | 0.165 | 16091.5 |
| indra | 3 | 9 | 0 | 5.6890 | 12.5523 | 1.166 | 1994.9 |

[Summary](../benchmarks/results/public-2/summary.json) · [Every request](../benchmarks/results/public-2/requests.jsonl) · [Resources and actual batch shapes](../benchmarks/results/public-2/cohorts.json)

| Engine | Per-run pages/s | Observed peak sum RSS MiB | Observed CPU s per cohort | Startup/readiness ms per run |
| --- | --- | --- | --- | --- |
| tesseract-distro | 0.445, 0.464, 0.434 | 85.0, 84.9, 84.8 | 16.74, 16.20, 13.32 | 0.0, 0.0, 0.0 |
| tesseract-best | 0.164, 0.168, 0.164 | 103.8, 102.8, 102.7 | 48.66, 45.36, 34.44 | 0.0, 0.0, 0.0 |
| indra | 1.064, 1.353, 1.119 | 527.2, 543.6, 506.9 | 9.88, 6.97, 7.04 | 1924.1, 1422.8, 1863.6 |

RSS is sampled across the whole process tree and double-counts shared mappings.
CPU/RSS observations include startup, warmup, measured work, and shutdown; they are not warm-only CPU/page.

Observed request tensors, including client warmups: indra: B1–B64, 11.80% padding.

## Remaining limits

- The initial `raw-2` run recorded a launcher affinity observation on the measurement CPU and was rejected.
  It was rerun after waiting for `taskset` to apply the workload mask before process sampling. The valid run is `raw-2-verified`.
- The initial `threads-2` experiment had only two-thread candidates. Use `threads-2-completed` for the matched one/two-thread controls and its disclosed recovery.
- No unmeasured 200-DPI-versus-300-DPI rendering conclusion is claimed. Source images contain both DPI variants; Tika's primary OCR rendering was 300 DPI.
- Native process affinity was tested. OCI root-filesystem correctness checks are separate evidence; the read-only cgroup hierarchy prevented actual application containers. No Docker/Kubernetes resource-enforcement or signed-image runtime performance result is claimed.
- Local Tika plans used 256 MiB parent / 512 MiB forked JVM heap ceilings and `ActiveProcessorCount` equal to the CPU budget, identically for both engines. They differ from deployment defaults (128 MiB parent and processor-count 1); these are not exact deployment timings.
- Raw Tesseract's startup column is zero because there is no separately started resident service; each request latency includes process/model initialization. Restarted services do not imply cold filesystem caches.
- Each cohort received one client warmup request. A format first encountered during the measured corpus may still trigger lazy library initialization; these short runs do not prove a fully saturated steady state.
- All p95/p99 numbers have small-sample warnings. No open-loop overload or long soak result is claimed.
- Use a held-out, manually adjudicated corpus and reserved hardware for a production acceptance gate.

See [methodology and primary references](benchmark-methodology.md) and [reproduction instructions](../benchmarks/README.md).

## Evidence package

The repository includes the exact synthetic fixtures and their hash manifest, frozen measured backend
source/model manifest, the initial harness snapshot, current harness, raw synthetic outputs, numeric public
observations, excluded-run numeric evidence, and the thread-ablation recovery provenance.
[Evidence index](../benchmarks/results/index.json) · [Synthetic manifest](../benchmarks/results/synthetic-fixtures/manifest.json) ·
[Frozen source hashes](../benchmarks/results/frozen-source-hashes.json) · [CPU topology](../benchmarks/results/ocr-host-topology.json)
