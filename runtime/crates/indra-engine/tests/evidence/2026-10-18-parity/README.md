# Fresh linked-engine acceptance, 18 October 2026

The corrected static engine passed all 14 prespecified implementation cases: exact UTF-8 output, identical captured and uncaptured output, and all 850 tensor comparisons across 163,005,555 elements. These are correctness diagnostics, not an estimate of OCR quality or a performance comparison.

`summary.json` records the aggregate; `results.json` preserves every case outcome. `tensor-comparisons.jsonl` contains all tensor names, shapes, dtypes, file hashes, error magnitudes and acceptance outcomes. Numerical array equality does not require identical `.npy` headers or file hashes.

## Identity and distinct gates

| Gate | Evidence | Result |
|---|---|---|
| Native build and execution | [Workflow 39092252965](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39092252965), artifact 12123143545; `native-acceptance.json` | Static GNU/glibc ELF; no interpreter or shared-library dependency; real scratch execution and verified model-cache TLS acquisition/reuse |
| Hosted native tests | Captured per-budget status files and raw log hashes | 23 unit tests and three native tests passed separately at actual `cpu.max=100000 100000` and `200000 100000`; each parent memory cap was 3 GiB |
| Fresh Python/Rust parity | Frozen `protocol.json`, `selection-plan.json`, reference and native environments | 14/14 cases; no failed tensor or text gates |

The OCR executable SHA256 is `d6217dcb6adbae36dc0f3c9af3a0804e83c81dc5af1ce46ba85c290cb360f64b`. The downloaded artifact ZIP SHA256 is `d9098c4c62a1f36226d8578fccc422aed903d492386ec8a3b11ebf63f4c9c219`. The parity result file SHA256 is `ce058c0b555a4ca1d2588b66c386edc594aa91f5aa37e72fc6e480f6e3d0c39f`.

The first linked build exposed a process-wide ONNX Runtime initialization error when a second engine was loaded. The production fix synchronizes one fallible environment initialization and retains that environment for the process lifetime. The corrected build passed concurrent initialization, subsequent reuse, and the complete native gates before this fresh parity rerun. The earlier failed build is not silently counted as a pass.

## Prespecified comparison

Selection preceded reference inference. The comparison protocol and harness were frozen before any new native predictions, including the first linked attempt. Protocol SHA256: `903a8a82a8ad9957cbe614dc4395e40db9065da200fe153a10aed91158f4ca68`.

The reference is Indra 0.3.2 at commit `3d773aeedc5d79d1d0a2a04007a9075cd71a59f4`, model revision `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`, and the exact 23-package hash lock. Both arms use FP32 CPU inference, one ORT/OpenCV thread, batch cap 64, adaptive detection, no table heads, explicit spinning, and disabled memory patterns/CPU arena. The environment file discloses the small session-options adapter needed to expose equivalent Python settings. Plain-versus-captured runs check that diagnostic instrumentation did not change output.

The cases cover PNG, JPEG, every TIFF frame, skew, quarter-turn orientation, German historical print, Greek legislation, Bengali, Devanagari, Chinese routing, Latin and French handwriting, and an explicit synthetic math-on/off pair. A routing test can pass despite poor OCR quality: the Chinese case has a three-byte output in both arms. Transport and synthetic fixtures, a rotated derivative, and math variants are not independent quality samples.

The gate requires exact tensor execution order, dtype and shape. Integers, booleans and preprocessing pixels/inputs are exact. Other floating tensors use the frozen combined tolerance `abs(native-reference) <= 1e-5 + 1e-5*abs(reference)`; box coordinates use `atol=1e-3, rtol=1e-5`. Skew has a separate `1e-12` absolute tolerance. NaN and infinity fail. There is no sorting, reshaping, dtype conversion or post hoc threshold adjustment to obtain a pass.

546 tensors used exact-value mode; 805 of all 850 tensors had identical values. The largest absolute difference was `1.71661376953125e-5` in German reading-order pairwise logits, within the combined absolute/relative tolerance. It would be incorrect to describe every floating tensor as bitwise identical.

## Math termination coverage

The hosted B1/B8/B64 gate repeats one independently declared equation across 64 rows and verifies actual encoder shapes, every row's text and unused metadata. Its deliberate repetition tests cache/batch correctness; it does not constitute 64 quality samples.

The separate, prespecified full-page math case exercises six heterogeneous rows in one native batch. `math-termination.json` derives coverage from the already captured tensors: first completion steps are 8, 32, 8, 27, 60 and 23. Five rows finish on EOS; one finishes through the reference low-confidence rule. There are 52 steps with both finished and unfinished rows. All completion masks and next-token arrays match the reference exactly, and finished masks never revert. This coverage analysis did not run another model or change a fixture.

## Reproduction and limits

`recorded-harness/` preserves the exact reference-only Python harnesses bound by the protocol. They are audit copies with the original acquisition paths, not production runtime code or relocatable installed commands. Reproduction in another workspace must explicitly map those paths, preserve every selected input/model byte, and record the new harness identity and attempt directory. Do not rewrite this frozen record.

The local parity host did not attest a one- or two-CPU cgroup budget. Its timing files must not be promoted into performance results. The separately hosted native tests did attest those CPU budgets, but test-suite duration is also not an OCR benchmark. Full tensor arrays and unfiltered process streams remain in the originating attempt directory; this compact repository record retains their hashes and every comparison result. Numerical limits, the selected paths and the distinction between conformance and quality remain material even after every gate passes.
