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

The 0.3.2 port passed a real static GNU/glibc link, scratch-image TLS model acquisition and offline reuse, and (historically, [run 39092252965](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39092252965)) 23 unit plus three native acceptance tests under both actual one-CPU and two-CPU cgroup quotas; the current acceptance image runs the unit, native, allocations, continuous-plan, request-allocations and language-model parity gates listed below. The corrected 0.3.2 executable also passed the frozen 14-case reference comparison: exact final text and 850 intermediate tensors containing 163,005,555 elements met their prespecified checks ([recorded evidence](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/crates/indra-engine/tests/evidence/2026-10-18-parity/README.md)). The Indra 1.1 port re-freezes that comparison against indra-ocr 1.1.2 in [`tests/evidence/2026-10-21-parity-1-1`](../runtime/crates/indra-engine/tests/evidence/2026-10-21-parity-1-1/README.md); its README states exactly which arms ran on which host. Local differential comparisons establish implementation agreement; they are not quota-attested performance or representative accuracy measurements.

## Exact reference and supported path

The algorithm reference is [Indra OCR 1.1.2, commit `0e96b93`](https://github.com/datalab-to/indra/tree/0e96b93c6810f17cf8352acbb4487cc9e45b66d5). Every startup verifies all selected artifacts from [model snapshot `bd02167f`](https://huggingface.co/datalab-to/indra-ocr/tree/bd02167fdf2005a19962cc15188f8496a2c87d3b) (tag v1.1.0), including metadata, the three n-gram language models and the model licence, before opening native sessions. Vocabulary arrays are generated from that exact metadata; they are model data and retain the model licence. Character classes that the reference reads from CPython's `unicodedata` (bidi classes, combining marks, letters, decimal digits, East Asian width, upper case) are embedded as Unicode 15.1 range tables generated from CPython 3.13, so the port classifies text like the interpreter of the reference arm rather than like a crate's Unicode version.

| Stage | Implemented rule | Reference |
|---|---|---|
| Decode | Grayscale PNG, JPEG and every classic TIFF page; inspect dimensions and TIFF segment bounds first | [`read_images`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/pipeline.py) |
| Preview and deskew | The page shrunk once to at most 1,600 pixels a side; the skew search scores the horizontal ink profile's row-to-row change at 2° steps over ±45° on a 400-pixel ink thumbnail, refines 0° and the two best coarse angles in 0.5° steps and the winner in 0.1° steps on the 800-pixel thumbnail; tilts below 0.3° are left alone | [`estimate_skew`, `preview`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/pipeline.py) |
| Orientation | 768-pixel square ink map of the straightened preview; five classes; the most likely turn only above probability 0.95; class 4 marks a page set in vertical columns; after a quarter turn the skew is measured again on the turned preview | [`orient_input`, `orientation`, `_turn`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/pipeline.py) |
| Detection | FP32 detector; four-connected components; adaptive second pass up to 2,048 pixels when at least eight detected lines have median input height below nine pixels | [`det_post.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/det_post.py) and `detect` |
| Crops | Boxes padded 0.3 heights left and right; the base recogniser reads a three-band window (1.6 heights above and below, 96 rows), classifiers and group recognisers the line band (0.2 heights, 32 rows); aspect-preserving width bounded at 4,096; light-on-dark lines (2nd/50th/98th histogram percentiles) are read inverted | [`rec_preproc.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/rec_preproc.py) |
| Routing | `script.onnx` softmax over base, CJK, RTL and world; a line goes to its group recogniser and, when the top probability is not above `route_sure` (0.95), to the base recogniser too; the reading with the strictly higher mean peak confidence wins; a vertical page sends every line to the CJK recogniser | [`line_groups`, `recognize`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/pipeline.py) |
| Recognition | CTC blank and repeat removal; mean retained-peak confidence threshold 0.5; a reading with confidence in `[0.5, gate)` of a recogniser with a language model is re-decoded by the prefix beam search (beam 8, top-8 classes, candidate floor 3e-3, blank skip 0.999, `logP_ctc + alpha·logP_lm + beta·tokens`; base alpha 0.5 beta 1.5 gate 0.9, CJK and world alpha 0.2 beta 1.5 gate 0.95); x positions follow first-emission steps | [`lm.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/lm.py), [`vocab.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/vocab.py) |
| Optional math | Classify lines on the same band crops; encode 64-pixel crops; initialize cross-attention caches; autoregressive typed KV caches; at most 319 steps; low-confidence early stop; replace text only when output contains `$` and mean token probability is at least 0.9 | [`math.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/math.py) |
| Assembly | Group characters joined with whitespace collapsed, NFC-composed (Hangul jamo) and, with right-to-left characters, flipped to logical order by the page direction (more right-to-left than left-to-right letters); split merged columns at token gaps and corroborated gutters; leader-only lines dropped and lone list markers joined to their items; reading order on the mirrored boxes of a right-to-left page; XY-cut blocks on a vertical page; otherwise average-linkage layout groups, geometric order inside groups and pairwise insertion-search order between groups | [`assemble`, `tidy_lines`, `rtl.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/pipeline.py), [`order_post.py`](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/order_post.py) |

The selected models use FP32, CPU execution, adaptive detection, `table_heads=False`, the language models on (`engine.lm`) and `engine.route_sure` 0.95; math is enabled in every deployment profile. There is no mask model or INT8 recognizer in the locked snapshot. PDF text-layer extraction, PDFium, Markdown and table rendering are outside this text adapter. Multi-page output preserves the existing adapter boundary `\n\f\n` between pages.

## Indra 1.1 behaviour

- **Dual read.** The script classifier decides a group for every line, but only a probability above `route_sure` lets a line skip the base recogniser. An unsure line is read twice and the reading with the strictly higher mean confidence is kept; ties keep the first (base) reading. `OcrStats.dual_read_lines` counts the extra reads.
- **Language-model gate.** Each of the base, CJK and world recognisers has an n-gram backoff model (`lm.npz`, `lm_cjk.npz`, `lm_world.npz`, about 107 MB decompressed, loaded once at startup into sorted per-order key and value tables). A greedy reading with mean confidence at or above 0.5 but below the group's gate is re-decoded by the CTC prefix beam search in fixed worker scratch; the beam search never allocates per line. The RTL recogniser has no language model. `OcrStats.lm_lines` counts rescored readings; `engine.lm = false` skips loading the tables and the rescoring.
- **Normalisation.** Group-recogniser and math lines are NFC-composed through the ICU normaliser locked for the runtime; its iterator holds up to seventeen pending combining marks inline, so only a line with eighteen or more consecutive marks would make it allocate. The allocation gate exercises a seventeen-mark run.
- **Right-to-left pages.** Group recognisers emit characters in visual order. `rtl::flip` restores logical order with the reference's subset of UAX #9 (clusters keep their combining marks, left-to-right runs keep their order, mirrored brackets swap). The page direction is decided from the letter counts of every line before splitting; a right-to-left page reads its right column first because the order features see the boxes mirrored.
- **Vertical pages.** Orientation class 4 turns the page a quarter turn counter-clockwise, measures the skew again on the turned preview, sends every line to the CJK recogniser and replaces the order model's decision by XY-cut blocks (a cut needs a gap of at least one median line height). The diagnostic trace maps the final boxes back to the page as decoded. `OcrStats.vertical_frames` counts such frames.
- **Deskew.** The search range is ±45° with the reference's three-stage schedule; the orientation model reads the straightened preview, and a quarter turn re-measures the angle. Exactly tied integer sharpness scores between coarse angles are resolved toward the later angle, where NumPy's unstable sort leaves the order undefined.

## Batching and memory

Text and classifier batches use the reference width policy: sort narrowest first by the one-band crop width, at most 64 rows, a widest-to-first width ratio of 1.3 after a minimum effective width of 64, and at most `64 × 640` effective pixel columns. Every native input is then right-padded to a multiple of eight. The public batch-size setting can lower the row cap. Math uses the reference wider ratio of four and its separate area cap. The base recogniser's tall-window tensors use the same row groups but their own padded width; that arena starts at 1.25 times the band arena and grows only at a new high-water mark (bounded by 64 rows of 4,096 columns), which is logged.

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

Run the following commands inside the GNU/glibc native builder described in [native-build.md](native-build.md), from `/app`, with the verified model cache mounted at `/models`. The builder installs `/native/cargo-env.sh`; that path is not exported to the host. Its ELF checks reject an interpreter or dynamic dependencies. A successful `DOCS_RS=1` check uses documentation bindings and remains a separate, weaker gate than the real linked tests.

```sh
. /native/cargo-env.sh
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

The acceptance image also runs, on the exact static test binaries under each CPU cap, the `lm-parity` gate (the ignored tests of `tests/lm_parity.rs`: 11,400 sampled `logp_keys` values and the 15 recorded `beam_search` calls must be bit-exact against the locked archives), `continuous-plan`, which executes the fixed-array selection source, and two allocation gates. `allocations` proves the language-model, bidi and NFC paths and the declared control operations (width planning, tensor scan, continuous owner selection) make zero Rust allocator calls. `request-allocations` runs the warmed request path on five PNG and TIFF fixtures with and without math, records the six allocator counters per request as JSON evidence, and fails unless they are identical across repeated requests, every allocation is freed, and no counter exceeds the committed per-fixture ceiling in `tests/request_allocations.rs`. That ceiling is the measured floor of ort 2.0.0-rc.14's per-value wrappers, not engine state: each input built with `TensorRef::from_array_view` allocates an `Arc<ValueInner>`, each session output an `Arc<AllocatorHandle>` and an `Arc<ValueInner>` (ort `src/value/mod.rs`), a session with more than four outputs (the recognisers have six) spills its output name, value and pointer lists out of their four-element `SmallVec`s (`src/session/mod.rs`, `src/util/stack.rs`), and the math decoder step's seven inputs and rank-5 cache tensors spill their input lists and shapes the same way. On `smoke` (one line, no reading-order run) that is 3 for orientation, 3 for detection, 4 for the script classifier and 18 for the base recogniser: the 28 allocations measured. Native allocations inside ONNX Runtime and OpenCV are not counted. Each worker logs one `cpu_dispatch` event with the instruction sets the standard library and OpenCV detected on the host, so a measurement can state which kernels actually ran on a portable build.

```sh
mkdir -p evidence
runtime/target/x86_64-unknown-linux-gnu/release/examples/ocr \
  backend/tests/data/smoke.png --model-dir /models --math \
  --output evidence/smoke.txt --stats evidence/smoke.jsonl
```

The first argument is the image path. `--threads`, `--cv-threads`, `--batch-size`, `--math`, `--lm`, `--route-sure`, `--intra-op-spinning` and `--inter-op-spinning` select the actual settings. `--warmup` executes all enabled native sessions before timing. `--expect reference.txt` compares final bytes exactly. Diagnostics use `--trace-directory EMPTY_DIRECTORY`; normal measurements leave it unset. Tensor exports can contain document content and belong with explicitly selected test fixtures, not routine production logs.

JSONL measurements include wall time, all seven stage times, frame/line counts, the complete text-batch histogram, actual valid versus padded columns and effective thread/spinning settings. Classifier/math calls are excluded from the text histogram; their stages and shapes are recorded separately by tracing and optional tensor evidence. Native spans contain model, script, dimensions, batch size and counts. They never include recognized text, images, filenames or error strings; the server supplies the parent trace and configured OTLP/Sentry exporters.

## Reference comparison

Use the locked Python reference environment (indra-ocr 1.1.2, the `backend/` image) only as the comparison arm, with `OrtOCR(model_dir, threads=N, rec_int8=False, adaptive=True, table_heads=False, math=...)` and `cv2.setNumThreads(N)`. The recorded harness in [`tests/evidence/2026-10-21-parity-1-1/recorded-harness`](../runtime/crates/indra-engine/tests/evidence/2026-10-21-parity-1-1/recorded-harness) wraps every session (orientation, detector, script classifier, the four recognisers with their blank/top-k outputs, order, math) and the turn, detection, darkness, routing and reading-order hooks, then `native_parity.py` executes the static `ocr` binary with `--trace-directory` and compares text and every tensor under the frozen rules. The native acceptance workflow runs both arms on the fixtures the repository can reproduce (transport fixtures, the synthetic math page, the two Hub example pages and the three CoreText-rendered RTL, world-script and vertical pages); the seven frozen corpus pages need the evaluation layer's acquisition tooling and are compared where those inputs are supplied. Read grayscale frames with `read_images`, evaluate each `Page.text`, and join pages with `\n\f\n`. Set OpenMP/BLAS helper thread limits to one and record them. If comparing altered batch size, spinning or memory-pattern settings, apply and record the same choices in both arms. The default public Python constructor does not expose every runtime control.

Compare preprocessing tensors before comparing text: decoded pixels, orientation inputs/results, deskew decisions, detector input/logits/boxes, dark-line flags, script probabilities and routing groups, per-recogniser inputs, CTC IDs/confidences/lengths and the blank/top-k log-probabilities that feed the beam search, math memory/logits/stop flags, and reading-order features/logits/permutation. Preserve the exact input/model hashes and effective native build configuration. NumPy's unstable default sort does not promise the order of equal keys; this implementation uses the original index to break ties deterministically. Float reductions and BLAS implementations can differ near rounding boundaries. Report every difference and both absolute/relative tolerances; identical output alone is a weaker gate.

In the recorded 0.3.2 rerun, 546 tensors used exact checks and 304 used numeric checks; 805 of all 850 tensors nevertheless had exactly equal element values. The largest absolute difference was approximately `1.7166e-5` in German reading-order logits, which passed the frozen combined absolute/relative tolerance (`atol=1e-5`, `rtol=1e-5`). Detector coordinates had a separately prespecified tolerance. The 1.1 evidence directory records the same kind of result for the 1.1.2 reference, states which arm ran on which host, and keeps the language-model fixtures (`tests/data/lm`) that were checked bit-for-bit on the development host. No tolerance was relaxed after seeing results. Implementation cases include transport variants and synthetic inputs and do not constitute independent accuracy samples.
