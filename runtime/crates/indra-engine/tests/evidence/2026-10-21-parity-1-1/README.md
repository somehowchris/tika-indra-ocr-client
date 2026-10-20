# Indra OCR 1.1.2 parity re-freeze, 20–21 October 2026

This directory re-freezes the implementation comparison of the native engine
against the unmodified Python reference, now indra-ocr 1.1.2 (upstream commit
`0e96b93c6810f17cf8352acbb4487cc9e45b66d5`) on the model snapshot
`bd02167fdf2005a19962cc15188f8496a2c87d3b` (tag v1.1.0). It is a correctness
diagnostic, not an estimate of OCR quality or a performance comparison.

## What ran where

| Arm | Host | Record |
|---|---|---|
| Python reference (`recorded-harness/reference.py`) | macOS arm64 development host; CPython 3.13.15, NumPy 2.5.4, ONNX Runtime 1.31.0, OpenCV 5.0.0.93, indra-ocr 1.1.2 from the hash-locked wheel, one ORT/OpenCV thread | `reference-macos-arm64/`: plan, environment, per-case text, details, tensor manifest (names, shapes, dtypes, SHA-256 of every `.npy`), results; raw arrays stay in the originating attempt directory |
| Frozen protocol (`native_parity.py freeze`) | same host, before any native prediction | `reference-macos-arm64/parity-protocol.json` (SHA-256 `51800b8caf68fb6d9c1d1633d5cbfaa594547d599aa5a923396fddde3838563d`) |
| Native engine (`native_parity.py run` on the static `ocr` binary) | **not executed on the development host**: the engine links ONNX Runtime and OpenCV through the GNU/glibc native builder, which this macOS arm64 host cannot run. The native acceptance workflow (`.github/workflows/native-engine.yml`) records the reference inside the Python backend image on the cases it can reproduce (not the seven corpus pages), freezes its own protocol and runs the comparison on the hosted runner; its artifact holds the native record | pending the hosted run |
| Language-model and beam-search parity | development host, native-free harness of `lm.rs` (same sources) | `tests/data/lm/`: 11,400 `logp_keys` values and 15 recorded `beam_search` calls, bit-exact; the acceptance image runs them as the `lm-parity` gate |

## Cases

All nineteen planned cases ran on the development host: the fourteen cases of
the 2026-10-18 comparison (four transport fixtures, seven frozen corpus pages,
the rotated Greek derivative, the synthetic math page with and without math)
and five new ones: the two Hub example pages (handwriting, Korean) and the
three CoreText-rendered pages in `fixtures/` (RTL, world scripts, vertical
CJK). The seven corpus pages were reconstructed byte-exact (every frozen
SHA-256 matched) with the evaluation layer's `indra-evaluate fetch` and
`prepare` built from `origin/consolidated/03-evaluation` (`bbe774f`); the
EUR-Lex page rendered with Poppler 26.09.0 matched the bytes frozen with
26.05.0. One documented publisher route (the German GDPR PDF, not used by any
case here) answered 403 and was skipped.

| Case | Purpose | Output bytes | Tensors |
|---|---|---:|---:|
| smoke | PNG decoding; text unchanged from 0.3.2 (`ec97908e…`) | 13 | 22 |
| two-pages | TIFF frames and page separator; unchanged (`b98d0cd9…`) | 33 | 44 |
| jpeg, skew | decoder and deskew parity (skew −2.0°); both unchanged (`9bffdd32…`) | 744 | 49 |
| bhl-deu-0018 | German historical print, Latin | 4,178 | 60 |
| eurlex-ai-act-el-p0002 | Greek legal page | 11,237 | 82 |
| bongabdo-img_20230101_0016 | Bengali handwriting; the 1.1 orientation model turns it by 180° | 734 | 64 |
| sankaracarya1925-03-body | Devanagari crop (skew −1.1°) | 3,977 | 57 |
| jingbao-1930-500 | Traditional Chinese newspaper crop | 184 | 62 |
| AMSMB_1-3-100 | Latin notarial charter, handwritten (skew 1.0°) | 1,495 | 86 |
| lectaurep-riant-01 | French notarial register (skew −0.8°) | 1,359 | 124 |
| rotated-greek | clockwise quarter turn of the Greek page; 1.1.2 leaves it unturned (class probability under 0.95), so the reference reads 80 bytes where 0.3.2 recovered the page | 80 | 65 |
| math-off / math-on | classifier and autoregressive math path | 64 / 281 | 76 / 355 |
| hf-handwriting | base recogniser with `lm.npz` rescoring | 620 | 81 |
| hf-korean | CJK routing, jamo composition, dual reads, `lm_cjk.npz` | 2,934 | 149 |
| synthetic-rtl | RTL recogniser, page direction (`page_rtl` true), flip, mirrored order features | 846 | 32 |
| synthetic-world | world recogniser with `lm_world.npz` | 1,449 | 46 |
| synthetic-vertical-cjk | orientation class 4, CJK columns, XY-cut blocks | 966 | 31 |

1,534 tensors in all. The rotated-Greek row is a property of the 1.1.2
reference, recorded as such; the native arm must reproduce it, and the
comparison does not judge whether it is desirable. The synthetic pages were
rendered with the committed Swift sources (`fixtures/render-pages.swift`,
`fixtures/render-vertical.swift`) and re-encoded as 8-bit grayscale PNG; their
SHA-256 values are bound in the plan. The vertical page is classified vertical
by `orient.onnx` with probability 0.961. Rendered pages exercise the
implementation paths; they are not quality samples, and the vertical page reads
poorly in both arms by construction.

## Rules

The comparison rules are those of the 2026-10-18 protocol: exact UTF-8 text,
captured and uncaptured native text identical, identical tensor sequence,
exact preprocessing inputs (now including the RTL and world inputs), exact
integers and booleans (CTC ids and lengths, top-k class ids, routing groups,
dark-line flags, permutations), `atol=1e-5, rtol=1e-5` for other floats,
`atol=1e-3` for boxes, `1e-12` for the skew angle. Native-only tensors
(`line-indices`, `order-blocks`, `page-boxes`) are excluded from the sequence
gate. No tolerance was relaxed after seeing results, and none may be.

## Reproduction

```sh
OMP_NUM_THREADS=1 OMP_THREAD_LIMIT=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1 \
NUMEXPR_NUM_THREADS=1 VECLIB_MAXIMUM_THREADS=1 \
python recorded-harness/reference.py --repo REPO --models MODELS \
  --upstream UPSTREAM --assets ASSETS [--corpus CORPUS] --output OUT/initial
python recorded-harness/native_parity.py freeze --reference OUT/initial
python recorded-harness/native_parity.py run --reference OUT/initial OCR_BINARY MODELS OUT/native
```

`UPSTREAM` holds the 18 `indra_ocr/*.py` files of the pinned commit
(`recorded-harness/upstream-sources.sha256`); `ASSETS` the two Hub example
pages (hashes in the harness); `MODELS` the verified 19-file cache. The
`.github/workflows/native-engine.yml` steps show the exact hosted invocation.
