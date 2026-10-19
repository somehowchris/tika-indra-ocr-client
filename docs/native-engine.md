# Native image OCR engine

`indra-engine` implements the selected Indra image-to-`Page.text` path in Rust. The HTTP worker owns one synchronous engine and calls it from its dedicated worker thread. Tika owns PDF parsing and rasterization.

Embedding threads need an explicitly reserved **8 MiB stack**. The real service
gate exposed stack exhaustion on Rust's smaller default worker stack; the
standalone native executable had run on an 8 MiB main-thread stack. Fixed
metadata arrays and native call frames count toward that budget even though
large tensor arenas use native heap storage. Configure `std::thread::Builder`
with `.stack_size(8 * 1024 * 1024)` for an engine owner and retain that allocation
in the worker resource budget. The service's source fix and its real-process
verification are recorded with the lifecycle evidence.

The reconstructed engine has passed a real static GNU/glibc link, scratch-image TLS model acquisition and offline reuse, and 23 unit plus three native acceptance tests under both actual one-CPU and two-CPU cgroup quotas. The corrected executable also passed the frozen 14-case reference comparison: exact final text and 850 intermediate tensors containing 163,005,555 elements met their prespecified exact or numeric checks. [Recorded evidence](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/crates/indra-engine/tests/evidence/2026-10-18-parity/README.md) identifies the source, executable, models, environments, tolerances, first failed build and corrected rerun. Local differential comparisons establish implementation agreement; they are not quota-attested performance or representative accuracy measurements.

## Exact reference and supported path

The algorithm reference is [Indra 0.3.2, commit `3d773ae`](https://github.com/datalab-to/indra/tree/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4). Every startup verifies all selected artifacts from [model snapshot `ed345c7`](https://huggingface.co/datalab-to/indra-ocr/tree/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8), including metadata and the model licence, before opening native sessions. Vocabulary arrays are generated from that exact metadata; they are model data and retain the model licence.

| Stage | Implemented rule | Reference |
|---|---|---|
| Decode | Grayscale PNG, JPEG and every classic TIFF page; inspect dimensions and TIFF segment bounds first | [`read_images`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py) |
| Orientation | 768-pixel square ink map; apply the most likely quarter turn only above probability 0.95 | [`orient_input`, `orientation`, `upright`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py) |
| Deskew | 800-pixel thumbnail; coarse 0.5° and fine 0.1° horizontal ink-profile search; leave tilts below 0.3° unchanged | [`estimate_skew`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py) |
| Detection | FP32 detector; four-connected components; adaptive second pass up to 2,048 pixels when at least eight detected lines have median input height below nine pixels | [`det_post.py`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/det_post.py) and `detect` |
| Recognition | White-padded crop, 32-pixel height, aspect-preserving width bounded at 4,096; Chinese classifier routes to its own vocabulary/model; CTC blank and repeat removal; mean retained-peak confidence threshold 0.5 | [`rec_preproc.py`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/rec_preproc.py), [`vocab.py`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/vocab.py) and `recognize` |
| Optional math | Classify lines; encode 64-pixel crops; initialize cross-attention caches; autoregressive typed KV caches; at most 319 steps; low-confidence early stop; replace text only when output contains `$` and mean token probability is at least 0.9 | [`math.py`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py) |
| Assembly | Split merged columns at token gaps and corroborated gutters; average-linkage layout groups; geometric order inside groups; pairwise insertion-search order between groups | [`assemble`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py), [`order_post.py`](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/order_post.py) |

The selected models use FP32, CPU execution, adaptive detection and `table_heads=False`. There is no mask model or INT8 recognizer in the locked snapshot. PDF text-layer extraction, PDFium, Markdown and table rendering are outside this text adapter. Multi-page output preserves the existing adapter boundary `\n\f\n` between pages.

## Batching and memory

Text and classifier batches use the reference width policy: sort narrowest first, at most 64 rows, a widest-to-first width ratio of 1.3 after a minimum effective width of 64, and at most `64 × 640` effective pixel columns. Every native input is then right-padded to a multiple of eight. The public batch-size setting can lower the row cap. Math uses the reference wider ratio of four and its separate area cap.

After row selection, the core `BatchPlan` now checks aligned work against the
actual reusable tensor arena. It replaces the prior maximum-width scan; it does
not change the native grouping policy. The text arena includes alignment
headroom beyond the raw grouping limit. [Arithmetic contracts and integration
gates](batch-proof.md) distinguish source equivalence, solver evidence, native
tensor equality and measured scan cost.

The default path uses **dynamic line batches within one document**. Requests are admitted by the bounded server queue. A separately enabled [two-owner stateless batching pilot](continuous-batching.md) can combine script/CTC rows from two requests in one invocation; its native gates pass, while its completed synthetic performance comparison fails the promotion screen. The default and the frozen quality-study arms do not enable it. The autoregressive math decoder runs all rows in a selected batch until every row finishes or the token limit is reached, as the reference does. It does not insert new lines into an already running KV cache.

Rust keeps line indices, token histories, masks and routing decisions in fixed arrays. Reusable native matrices hold large tensors and reading-order scratch without constructing multi-megabyte stack arrays. The crop arena grows only when a larger crop is required. Script classification and recognition re-create crops in bounded batch scratch rather than retaining every full-page crop simultaneously. This trades additional preprocessing work for bounded crop storage; the measured outcome must be reported, not assumed faster.

ONNX Runtime's CPU arena and memory-pattern cache are disabled for variable shapes. The reference explicitly disables the CPU arena because its allocations grew with changing line batches. Native inference, OpenCV resizing/connected components, tensor handles, logging exporters and the allocator still perform allocations. This implementation makes no claim of zero allocations for OCR. `ort` receives borrowed tensor data; math cache outputs move as typed owned tensors. OpenCV's TIFF decoder API requires a native vector, bounded to one requested page; its returned matrix handle shares the decoded pixels without duplicating them.

OpenCV's thread setting is process-global and [must not be changed by concurrent threads](https://github.com/opencv/opencv/blob/40738fb16ceddb5fb3fea747585f7ce6abb0605b/modules/core/include/opencv2/core/utility.hpp). Engine startup configures it once under a lock. All engines in one process must use the same OpenCV thread count; a conflicting setting fails explicitly. Request-local decoder errors reject that document, while native allocation and model/runtime failures retain their distinct terminal error classification.

ONNX Runtime also uses one process-lifetime environment. Engine startup shares a fallibly initialized environment under a lock, while each session retains its explicit thread configuration. The first real native acceptance run exposed repeated environment initialization when loading a second engine; the corrected build adds concurrent initialization coverage and passes repeated native engine creation. This process-level requirement cannot be established by a documentation-only type check.

| Boundary | Limit |
|---|---:|
| Encoded request | 20 MiB |
| Side length | 16,384 pixels |
| Decoded pixels across all pages | 25,000,000 |
| TIFF pages | 16 |
| Detected and final split lines per page | 1,024 |
| CTC timesteps per line | 512 |
| Response | 4 MiB UTF-8 |
| Optional tensor evidence | 512 MiB per engine |

Animated PNG and BigTIFF are explicitly rejected. Duplicate PNG dimensions, APNG control chunks, duplicate TIFF dimension/segment tags, cyclic page directories and truncated segment tables are rejected before decoding. A later-page failure clears accumulated output and returns an error. Deadlines are checked between native calls and during bounded CPU loops. A synchronous native call is not safely interrupted by returning a timeout to a client; the server retains its capacity until native return or its terminal watchdog action.

## Development and acceptance

Use the GNU/glibc native builder in [native-build.md](native-build.md). Its ELF checks reject an interpreter or dynamic dependencies. A successful `DOCS_RS=1` check uses documentation bindings and remains a separate, weaker gate than the real linked tests.

```sh
. /opt/indra-native/cargo-env.sh
cargo test --locked --manifest-path runtime/Cargo.toml \
  --target x86_64-unknown-linux-gnu -p indra-engine --lib
cargo clippy --locked --manifest-path runtime/Cargo.toml \
  --target x86_64-unknown-linux-gnu -p indra-engine --all-targets -- -D warnings
cargo build --locked --manifest-path runtime/Cargo.toml --release \
  --target x86_64-unknown-linux-gnu -p indra-engine --example ocr

INDRA_TEST_MODEL_DIR=/models cargo test --locked \
  --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
  -p indra-engine --test native -- --ignored --test-threads=1

INDRA_TEST_MODEL_DIR=/models INDRA_TEST_THREADS=1 cargo test --locked \
  --manifest-path runtime/Cargo.toml --target x86_64-unknown-linux-gnu \
  -p indra-engine --lib -- --include-ignored --test-threads=1
```

Acquire the immutable model cache explicitly before these commands; serving never downloads. The ignored integration tests require real libraries and the selected model files. They test the supplied PNG text, both distinct TIFF pages, deadline/truncation cleanup, reuse after errors, and every optional math model, including real decoder cache steps. The ignored unit gate compares real decoder execution at batch caps one, eight and 64 on 64 repeated equations. A separate frozen six-row math parity case has five EOS terminations and one low-confidence termination, with 52 steps containing both finished and unfinished rows; every stop mask and next-token array matches the reference. These synthetic inputs test batching and termination, not scientific-document accuracy. Running the commands in an unrestricted shell does not attest a one-CPU or two-CPU budget.

```sh
runtime/target/x86_64-unknown-linux-gnu/release/examples/ocr \
  backend/tests/data/smoke.png --model-dir /models \
  --output evidence/smoke.txt --stats evidence/smoke.jsonl
```

The first argument is the image path. `--threads`, `--cv-threads`, `--batch-size`, `--math`, `--intra-op-spinning` and `--inter-op-spinning` select the actual settings. `--warmup` executes all enabled native sessions before timing. `--expect reference.txt` compares final bytes exactly. Diagnostics use `--trace-directory EMPTY_DIRECTORY`; normal measurements leave it unset. Tensor exports can contain document content and belong with explicitly selected test fixtures, not routine production logs.

JSONL measurements include wall time, all seven stage times, frame/line counts, the complete text-batch histogram, actual valid versus padded columns and effective thread/spinning settings. Classifier/math calls are excluded from the text histogram; their stages and shapes are recorded separately by tracing and optional tensor evidence. Native spans contain model, script, dimensions, batch size and counts. They never include recognized text, images, filenames or error strings; the server supplies the parent trace and configured OTLP/Sentry exporters.

## Reference comparison

Use the locked Python reference environment only as the comparison arm, with `OrtOCR(model_dir, threads=N, rec_int8=False, adaptive=True, table_heads=False, math=...)` and `cv2.setNumThreads(N)`. Read grayscale frames with `read_images`, evaluate each `Page.text`, and join pages with `\n\f\n`. Set OpenMP/BLAS helper thread limits to one and record them. If comparing altered batch size, spinning or memory-pattern settings, apply and record the same choices in both arms. The default public Python constructor does not expose every runtime control.

Compare preprocessing tensors before comparing text: decoded pixels, orientation inputs/results, deskew decisions, detector input/logits/boxes, script decisions, CTC IDs/confidences/lengths, math memory/logits/stop flags, and reading-order features/logits/permutation. Preserve the exact input/model hashes and effective native build configuration. NumPy's unstable default sort does not promise the order of equal keys; this implementation uses the original index to break ties deterministically. Float reductions and BLAS implementations can differ near rounding boundaries. Report every difference and both absolute/relative tolerances; identical output alone is a weaker gate.

In the recorded corrected rerun, 546 tensors used exact checks and 304 used numeric checks; 805 of all 850 tensors nevertheless had exactly equal element values. The largest absolute difference was approximately `1.7166e-5` in German reading-order logits, which passed the frozen combined absolute/relative tolerance (`atol=1e-5`, `rtol=1e-5`). Detector coordinates had a separately prespecified tolerance. The evidence records complete tensor shape, dtype, order, hashes and comparison outcomes; no tolerance was relaxed after seeing results. Fourteen implementation cases include transport variants and synthetic inputs and do not constitute 14 independent accuracy samples.
