# Indra OCR: checked findings and implementation decisions

Reviewed 2026-10-17 against upstream code and model snapshots. Exa searches requested 25 results across four queries (10 + 5 + 5 + 5); known primary URLs were fetched separately. This count is search coverage, not a count of independent benchmark replications. Third-party summaries with unsupported language, INT8 and API claims were excluded.

## Current pin: Indra OCR 1.1 (checked 2026-10-21)

The native engine and the Python reference arm now pin Indra OCR 1.1. The sections after this one record the 2026-10-17 review of 0.3.2, and their measurements describe that revision.

| Item | Verified value | Primary evidence |
|---|---|---|
| Upstream code | `indra-ocr==1.1.2`, commit `0e96b93c6810f17cf8352acbb4487cc9e45b66d5`, dated 2026-10-20 | [Package metadata at code commit](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/pyproject.toml) |
| Model commit | `bd02167fdf2005a19962cc15188f8496a2c87d3b`, Hub tag `v1.1.0`. Packages 1.1.1 and 1.1.2 ship the same models and change only PDF text-layer handling, which Tika's rendered-image path does not use | [Changelog at code commit](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/CHANGELOG.md), [immutable Hugging Face metadata](https://huggingface.co/api/models/datalab-to/indra-ocr/revision/bd02167fdf2005a19962cc15188f8496a2c87d3b?blobs=true) |
| Fetched bundle | 19 files, 95,979,210 bytes; the five math files are 19,874,159 bytes of that | File hashes in `backend/models.lock.json` |
| Model license | Byte-identical to the 0.3.2 pin's `LICENSE` (14,750 bytes, SHA-256 `50c88a01…0a5f`) | Both lock files |

The 1.1 pipeline adds four script groups with a second reading by the base recognizer when the script classifier is below 0.95 certainty, a crop three line heights tall for the base recognizer, inverted reading of light-on-dark lines, an n-gram language-model beam search for lines whose confidence lies in [0.5, gate), right-to-left logical order, vertical CJK pages, and deskew up to 45°. Each is ported in `runtime/crates/indra-engine`; the [1.1.2 parity re-freeze](../runtime/crates/indra-engine/tests/evidence/2026-10-21-parity-1-1/README.md) records how the port was compared with the unmodified reference.

Upstream reports, on its own benchmark harness, OmniParseBench 62.3 → 66.7 (71.4 with math), handwriting 8.3 % → 25.2 %, rotated pages 25.3 % → 50.3 %, and about 16 % more time per page than 0.3.2. This repository does not reproduce those figures; its own comparison is the frozen evaluation named in the [results ledger](../docs/results.md).

## Exact software and artifacts

| Item | Verified value | Primary evidence |
|---|---|---|
| Upstream | `datalab-to/indra`, Python package `indra-ocr==0.3.2` | [Package metadata at code commit](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/pyproject.toml) |
| Code commit | `3d773aeedc5d79d1d0a2a04007a9075cd71a59f4`, dated 2026-10-15 | [Commit](https://github.com/datalab-to/indra/commit/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4) |
| Model release | `v3.1.1`, same model weights as `v3.1` plus Hub metadata | [Changelog](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/CHANGELOG.md) |
| Model commit | `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8` | [Immutable Hugging Face metadata](https://huggingface.co/api/models/datalab-to/indra-ocr/revision/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8?blobs=true) |
| Standard ONNX files | `det`, `rec`, `order`, `orient`, `script`, `rec_cjk`; 18,158,743 bytes combined | Same model metadata; file hashes in `backend/models.lock.json` |
| Math ONNX files | `mathcls`, `math_enc`, `math_dec_init`, `math_dec_step`; 19,866,362 bytes combined | Same model metadata |
| Complete fetched bundle | 14 files, 38,187,650 bytes including JSON metadata and model license | Hash-verified by `backend/fetch_models.py`; arithmetic over the manifest |
| Optional but absent | No `mask.onnx` or `rec.int8.onnx` in the pinned published snapshot | Same model metadata and [loader](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py) |

The package is a Python pipeline around native ONNX Runtime/OpenCV/PDFium work. No official native Rust engine or production image-service API was found in the inspected repository. This implementation adds a bounded HTTP service and retains upstream model behavior. Tika integration uses the repository's static GNU/glibc Rust bridge; the Python and Java runtimes remain dynamic native runtimes inside distroless images.

## Accuracy: choose a workload-specific profile

OCR support is Latin (about 50 languages), Cyrillic, Greek, Devanagari, Bengali, and simplified/traditional Chinese. The Chinese recognizer does not provide Japanese kana or Korean Hangul OCR. Other scripts may survive a digital PDF's existing text layer, which is a separate path. Handwriting, degraded scans, vertical text and complex tables are documented limitations. [Upstream limitations and languages](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/README.md)

The standard CPU profile keeps FP32 recognition, orientation, adaptive detection, reading order and Chinese routing. `INDRA_MATH=1` adds the published math models. The publisher reports OmniParseBench headline 62.1→66.8, text 41.4→57.5 and math tests 14.3→56.5, while tables change 56.4→54.8. Those are benchmark test scores, not character-accuracy percentages. Their approximately 210→360 ms median comparison used four x86 cores; it is not a one- or two-vCPU measurement. [Release evidence](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/CHANGELOG.md)

There is therefore no single setting proven to maximize every kind of accuracy and throughput. Math is an explicit alternate profile; it needs table/numeric regression checks. Raising DPI, lowering rejection thresholds, quantizing models or replacing geometry-based table assembly is an experiment requiring rescoring, not a guaranteed upgrade.

The implementation does not pass PDFs to Indra's `auto` text-layer path. Tika owns PDF extraction and renders pages for the OCR bridge. Compare this integration with Indra's OCR-only results, and evaluate Tika's digital-PDF behavior separately. The publisher's auto-mode text score is 41.3; OCR-only is 33.5. Their Tesseract Latin-model row is 19.3. Default auto-mode success must not be represented as an image-OCR advantage. [Benchmark table](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/README.md)

## Actual batching and CPU execution

`OrtOCR.recognize(gray, boxes, batch=64)` receives one page's detected boxes. It groups lines by recognition script, makes width-homogeneous batches and scatters the readings back to original line indexes. The batching policy limits count, a ratio of floored widths, and a grouping proxy for padded work. Final alignment to eight columns happens afterward. This is dynamic line batching within a page. `convert_many` is page scheduling across worker processes. Neither is a cross-request tensor-batching server. [Recognition](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py), [batch construction](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/parallel.py), [tensor padding](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/rec_preproc.py)

| Property | Inspected behavior |
|---|---|
| Ordinary crop tensor | Float32 `[B,1,32,W]`, width aligned to eight |
| Ordinary batch limits | Count 64; ratio 1.3 after treating widths below 64 as 64; count × max(width, 64) ≤40,960 before final alignment; no 64-pixel tensor-width floor |
| Math classifier | Ordinary width-batch policy, independent default batch count |
| Math decoder | Crop height 64; width ratio 4; grouping budget 655,360 pixel columns before final alignment; model metadata allows 320 tokens |
| ORT threads | Explicit intra-op count; inter-op count one; graph optimizations enabled |
| ORT CPU arena | Disabled upstream because changing batch shapes caused retained-memory growth in its tests |
| Quantized recognition | Upstream can auto-select `rec.int8.onnx` if present; this backend forces `rec_int8=False` and rejects unlocked model files |
| Provider | Explicit `CPUExecutionProvider`; service verifies every required session uses it |

The math decoder caches attention keys/values and stops a line's decoding after sustained low confidence; a math result replaces normal OCR only when it contains math and reaches the configured confidence threshold. [Math implementation](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py)

Upstream `usable_cores()` counts CPU affinity and its memory helper reads host physical RAM. It does not enforce cgroup CPU quota or cgroup memory limits. Its small-job path can also use all configured threads in one process. The service instead uses explicit worker/thread limits, persistent processes, bounded admission and external whole-stack CPU constraints. [Parallel execution source](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/parallel.py)

ORT thread spinning is enabled by default in the runtime and can trade latency against CPU usage. One intra-op thread avoids additional session worker threads. Provider/thread/spinning changes should be compared at the same total CPU allocation. [ORT thread management](https://onnxruntime.ai/docs/performance/tune-performance/threading.html)

OpenVINO is an optional experimental candidate on suitable Intel hardware. Upstream accepts a provider list, but this backend intentionally fixes CPUExecutionProvider. ORT's OpenVINO documentation recommends different graph-optimization treatment and exposes independent thread/stream controls. A provider-name change alone does not establish correctness, dynamic-shape compatibility or a speed advantage. No OpenVINO performance claim is made here. [Official OpenVINO EP documentation](https://onnxruntime.ai/docs/execution-providers/OpenVINO-ExecutionProvider.html)

## What the published performance figures establish

The publisher's 49.4 pages/second result used 32 AMD EPYC CPU cores and 300 OmniParseBench pages in OCR mode, with one single-threaded worker per core. Indra averaged three runs; competing engines ran once. Model loading and one warm-up page per worker were excluded. The four-core figure was 7.3 pages/second. These data do not establish performance at one or two vCPUs and must not be linearly extrapolated into a deployment promise. [Datalab benchmark announcement](https://www.datalab.to/blog/indra-ocr)

The checked accuracy harness loads a persistent converter with `threads=1`, opens each page, obtains Indra output, and passes it to the benchmark's scorer. It writes per-page results and summary files. Reproduction still requires pinning the dataset/scorer and matching PDF text-layer access, DPI, languages, headers/footers and CPU limits across engines. [Checked benchmark harness](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/bench/opb_indra.py)

Local quantitative accuracy/throughput comparisons are supplied by the repository's benchmark workstream. This research workstream ran correctness and fault-recovery checks on synthetic images, without treating them as corpus-level accuracy evidence.

## Local proof completed in this workstream

| Check | Observed result | Scope |
|---|---|---|
| Exact dependencies | Python 3.13.16; Indra 0.3.2; ORT 1.31.0; OpenCV 5.0.0; NumPy 2.5.4; Pillow 12.1.1 imported successfully | Isolated environment, native Linux execution |
| Model integrity | All 14 locked files downloaded and SHA-256 verified | Immutable model commit above |
| Native + actual OCR smoke | Passed with math disabled and enabled | `backend/image_smoke.py`; same command embedded in final image build |
| PNG/TIFF OCR | Known invoice string recognized; both TIFF pages retained in order with form-feed separator | Synthetic fixtures in `backend/tests/data` |
| Invariants + HTTP | 12 tests passed, including pre-decode limits, corrupt models, bounded output, required/framed bodies and non-success errors | `backend/test_server.py` against a real worker |
| Diagnostic deadline | A bounded 16-frame TIFF hit a 0.5-second deadline: HTTP 504 in 0.518 s | Single diagnostic run on CPUs 4/5, not a throughput result |
| Successful replacement | Readiness restored after 0.925 s; subsequent small OCR succeeded in 0.126 s | Same diagnostic run; production deadline remains 60 s |
| Idle native child death | Test-owned SIGKILL caused service nonzero exit without any new OCR request | `backend/test_recovery.py` |
| Failed replacement | Missing recognition file in an isolated model directory caused replacement failure and supervisor nonzero exit | Original model files untouched; recovery suite passed both cases |
| Exact distroless filesystem | CPython 3.13.5, native imports and both final-stage OCR commands passed as UID/GID 65532 with a read-only root and 128 MiB `noexec,nosuid` temporary filesystem | Exported pinned OCI rootfs; [evidence](../validation/backend-rootfs.json); separate from native measurement Python 3.13.16 |
| Build validation defect | Replaced the math smoke's `runpy` invocation, which could not resolve sibling imports, with `image_smoke.py --math`; both final commands then passed | Exact final-root execution caught a real Dockerfile check failure |

An initial 0.1-second diagnostic deadline also killed the small follow-up image; that demonstrated an overly short budget, not recovery failure. The follow-up test used 0.5 seconds to distinguish large-request cancellation from successful small-request handling. The supervisor failure checks were added after identifying that readiness alone does not cause an orchestrator restart.

The math smoke loads the optional math profile and processes a synthetic invoice. It is not a separate equation-recognition accuracy test. The distroless check reproduced the build stages inside an exported Debian builder and executed commands in the exported final filesystem; it did not execute `docker build` or `docker run`, enforce CFS quotas, or run Kubernetes. Those distinctions remain explicit in the evidence.

### Bridge build-profile measurement: no OCR

A separate experiment ran three balanced replications of 100 serial calls for each of `release` (thin LTO) and `maxperf` (fat LTO), at one and two logical CPU limits. The stand-in, launcher and spawned bridge shared the same affinity. Each call sent a valid 256 KiB PNG and received 16 KiB of text. Timing included subprocess launch, loopback transport and output file creation/write/close; fixture generation, warmups and output reread/validation were excluded. This isolates adapter-path overhead and does not measure OCR or Tika. [Method and all samples](../validation/bridge-profile-samples.json), [statistics and binary fingerprints](../validation/bridge-profile-summary.json)

| CPU budget | Thin mean / p50 / p95, ms | Fat mean / p50 / p95, ms | Paired fat minus thin mean, ms, with 95% interval |
|---|---:|---:|---:|
| One logical CPU | 2.760 / 2.747 / 4.313 | 2.792 / 2.655 / 4.417 | +0.032 [−0.407, +0.471] |
| Two logical CPUs | 2.331 / 2.121 / 3.537 | 2.214 / 2.082 / 3.384 | −0.117 [−0.367, +0.133] |

All 1,200 measured calls and 120 excluded warmups passed byte-for-byte request/output validation. Intervals use the three replication means with Student t and two degrees of freedom. Both paired intervals include zero. The fat-LTO binary was 28,928 bytes smaller, but this evidence does not justify changing the thin-LTO default. Shared-host load, CPU frequency, Python launcher cost and warm filesystem cache limit interpretation; no `fsync` or physical durability was measured.

## Model licensing is a deployment constraint

Code is Apache-2.0. The pinned weights use a modified OpenRAIL-M license. The actual terms restrict commercial use above US$20 million prior-year gross revenue or total equity/debt funding, with stated personal/research exceptions for those thresholds. They also restrict use by an entity offering a competing product/service, and include attribution and share-alike language extending to output. Hosted access is included in the definition of distribution. Those terms deserve review against the intended entity and service; the code's Apache license does not replace the weights' terms. [Exact pinned license](https://huggingface.co/datalab-to/indra-ocr/blob/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8/LICENSE)

## Deployment decisions supported by this evidence

Keep Tika 3.3.2 responsible for document parsing and render policy; preserve the upstream Indra pipeline behind a small bounded service. Use the static GNU/glibc bridge for the narrow process/HTTP contract. Match the Debian/Python ABI when copying a virtual environment into distroless, preserve native library loader names, and execute final-image import/inference checks. [Distroless Python instructions](https://github.com/GoogleContainerTools/distroless/blob/main/python3/README.md)

Bake and verify the immutable model snapshot. The default upstream downloader rechecks a potentially movable tag daily; supplying a local verified directory removes that runtime variability. [Model downloader source](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/models.py)

Use the measured quality/latency tradeoff to select standard versus math mode and recognition batch size. A full Rust rewrite, cross-request crop scheduler, quantization, alternate provider or more workers requires a separate differential-accuracy and full-stack performance gate. None should be described as an improvement before those measurements exist.
