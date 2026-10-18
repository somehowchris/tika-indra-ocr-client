# Reproducible OCR report

Run from the repository root on Linux x86_64 with `uv`, `curl`, `tar` and `sha256sum`:

```sh
evaluation/report/render.sh
```

The script uses Python **3.12.11**, installs the complete **50-package hash-locked**
Jupyter/Seaborn environment, verifies the official Quarto **1.10.19** archive,
runs normal and optimized-Python admission tests, and renders self-contained
`report.html`. Tool identities live in `tools.json`. The interpreter is the
available stable 3.12 release actually used for this reconstruction; the missing
checkpoint's claimed Python 3.12.14 is not asserted as reproduced.

The current render contains the retained historical Python-Indra/Tesseract study,
frozen-corpus coverage and **32 explicitly unsubmitted cells**. It contains no
new native OCR measurement. Historical chart checks verify all sixteen bars,
including source identity, position, color, value and zero baseline. Seaborn
supplies plotting; Python serves reporting and the separate reference arm only.

The historical CSV was committed with LF records although its retained provenance
hash describes CRLF. `baseline-csv-transport.json` binds both exact byte identities;
reconstructing CRLF reproduces the original hash without changing one field. Raw
source summaries and frozen provenance remain unchanged. Historical validation
uses explicit exceptions and also runs under Python `-O`.

## Admit actual study artifacts

Download and verify the combined workflow artifact first. Select its expected
source commit independently, then render with both variables:

```sh
OCR_EVALUATION_ROOT=/absolute/path/to/ocr-evaluation-combined \
OCR_EVALUATION_SOURCE_SHA=<full-40-character-reviewed-source-commit> \
  evaluation/report/render.sh
```

`study.py` admits only the combined 32-cell/112-comparison schema produced by the
Rust execution workflow. It verifies exact source/corpus/protocol/seed identities,
complete file inventories, contained regular paths, shard/candidate/evaluator/OCI
bindings, raw outputs and journals, first-attempt metadata, 18-column attempt CSVs,
quality-summary arithmetic, complete-workload denominators and stable resource
ceilings. Partial outcomes remain visible and never supply completed measurements.
Malformed or misattributed artifacts abort rendering; no fallback hides them.

This reporting validator does not independently verify Cosign cryptography,
re-score OCR strings or certify live cgroup enforcement. The producing Rust
scorer, exact release signature verification and retained execution receipts are
separate required evidence. Counts must reconcile before conclusions; neither
hash integrity nor passing synthetic loader fixtures establishes OCR quality.

## Tests and publication gates

The dependency-free admission tests can run without Quarto or Seaborn:

```sh
python3 -B -m unittest discover -s evaluation/report -p 'test_*.py' -v
python3 -B -O -m unittest discover -s evaluation/report -p 'test_*.py' -v
```

`receipt.py` confirms every planned status is present in the actual HTML and writes
`render-evidence/receipt.json` binding source files, dependency lock, rendered HTML,
PNG/SVGs and exact environment. Regeneration resets visual approval to **pending**.
The reconstructed historical PNGs were inspected for legibility, labels, legends,
axis units and clipping. The complete HTML still needs independent visual review.

Remaining scientific publication gates are actual signed-image execution, reference
letter-count reproduction from acquired hash-verified references, measured-study
figures and paired interpretation, and an independent hash-bound full visual
review. Keep primary pages, challenges and the descriptive two-crop laboratory
supplement separate. Do not infer isolated CPU scaling from different hosted
machines or continuous batching from the serial frozen workload.
