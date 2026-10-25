# Reproducible OCR report

Run from the repository root on Linux x86_64 with the `uv` release pinned in
`tools.json`, plus `jq`, `curl`, `tar` and `sha256sum`:

```sh
evaluation/report/render.sh
```

The script uses Python **3.12.11**, installs the complete **50-package hash-locked**
Jupyter/Seaborn environment, verifies the official Quarto **1.10.19** archive,
runs normal and optimized-Python admission tests, draws every study figure once
from labelled synthetic rows so the Seaborn artist checks execute before the
real render, and renders self-contained `report.html`. Tool identities, including
the pinned `uv` release and the commit-pinned setup action, live in `tools.json`.
The hosted `report` job in [`ocr-evaluation.yml`](../../.github/workflows/ocr-evaluation.yml)
runs this script after aggregation, with or without admitted evidence, and
retains `render-evidence/`, `study-data/`, `report.html` and the committed figures.
A sealed measurement is re-rendered without re-measuring by dispatching the
registered `OCR evaluator` workflow (`gh workflow run ocr-evaluator.yml --ref
<branch> -f report_evidence=ocr-evaluation-<sha>-<run>-<attempt>-combined
-f report_run_id=<run>`): it calls this workflow's render-only mode, only the
report job runs, the receipt binds the measurement's own source and
study-index SHA-256, and the report artifact name carries the commit whose
report code rendered it. The interpreter is the
available stable 3.12 release actually used for this reconstruction; the missing
checkpoint's claimed Python 3.12.14 is not asserted as reproduced.

Without an admitted artifact, the render contains the retained historical study,
frozen-corpus coverage and **24 explicitly unsubmitted cells**. With admitted
evidence, Seaborn adds main-primary strict/collapsed CER, baseline-paired frozen
intervals, whole-main throughput/latency, separate challenge/laboratory quality
views and a descriptive main-primary strata view (strict and whitespace-collapsed
CER by appearance, script and domain, one colour per arm, faceted by CPU budget).
The workload view adds min-max whiskers of the three per-repetition throughput and
p50 values and a footer naming each budget's CPU model, AVX-512F flag, logical CPUs,
SMT siblings and host. The report then states the admitted release run, its
aggregation time and study-index SHA-256, and Results and Discussion sections whose
every number `analysis.py` reads from the exported tables at render time; the
report date is the render date. All plotted data and actual-artist checks are exported in `study-data/`. Historical chart checks verify all sixteen bars,
including source identity, position, color, value and zero baseline. Seaborn
supplies plotting; Python serves reporting and the separate reference arm only.

The historical CSV was committed with LF records although its retained provenance
hash describes CRLF. `baseline-csv-transport.json` binds both exact byte identities;
reconstructing CRLF reproduces the original hash without changing one field. Raw
source summaries and frozen provenance remain unchanged. Historical validation
uses explicit exceptions and also runs under Python `-O`.

The [always-math amendment](../audits/always-math-20261019/amendment.json) records why two text-only arms were removed after known quick diagnostics. It preserves the original protocol identities and changes no scored input, scoring rule, seed, repetition or resource limit. The [Indra 1.1 amendment](../audits/indra-1-1-20261020/amendment.json) binds the two Indra arms to model revision `bd02167f` (v1.1.0, indra-ocr 1.1.2) with math, LM beam search and dual-read routing, again without changing protocol bytes; `study.py` rejects an artifact unless every admitted shard's `provenance/source/backend/models.lock.json` carries that revision and package version.

## Admit actual study artifacts

Download and verify the combined workflow artifact first. Select its expected
source commit independently, then render with both variables:

```sh
OCR_EVALUATION_ROOT=/absolute/path/to/ocr-evaluation-combined \
OCR_EVALUATION_SOURCE_SHA=<full-40-character-reviewed-source-commit> \
  evaluation/report/render.sh
```

`study.py` admits only the combined 24-cell/60-comparison schema produced by the
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

`receipt.py` confirms every planned status is present in the actual HTML, that no
plotting object repr leaked into it, and, for admitted evidence, that it prints the
index's aggregation time; it writes
`render-evidence/receipt.json` binding source files, dependency lock, rendered HTML,
PNG/SVGs and exact environment; the hosted report job retains that receipt, so no
reconstruction receipt is committed. Regeneration resets visual approval to **pending**.
`test_visual_reviews.py` asserts that every committed PNG under `evaluation/figures/`
still has the SHA-256 (and bytes, where recorded) that its review receipt approved.
The reconstructed historical PNGs were inspected for legibility, labels, legends,
axis units and clipping. The complete HTML still needs independent visual review.

The receipt lists the scientific gates still open for its render. For the sealed
measurement of run 39810108196 (24 of 24 cells admitted) they are the independent
numerical replay, reference letter-count reproduction from acquired hash-verified
references, and an independent hash-bound visual review of the complete HTML; a
render with missing cells or no artifact also lists the signed-image matrix and the
result interpretation. Keep primary pages, challenges and the descriptive two-crop laboratory
supplement separate. Do not infer isolated CPU scaling from different hosted
machines or continuous batching from the serial frozen workload.

Study figures retain all arm labels, explicit missing values and failure/coverage counts. Pair intervals are copied from the frozen Rust output, never fitted by Seaborn. `paired.csv` exports all 60 planned pairs (`left`/`variant` columns) for all three frozen metrics (`metric`: `cer`, `cer_whitespace_collapsed`, `wer`); the paired figure itself stays the prespecified fast-PSM3 strict-CER view. The strata figure draws only `summaries.csv` rows for the main primary cohort, a filled strict and an open collapsed mark per arm joined by a line, at arm offsets the renderer keeps at least one mark diameter apart; it reports per-arm scored/selected, failures and metric limits in `plot-validation.json`, is descriptive and carries no interval. `performance-repetitions.csv` exports each cell's three per-repetition throughput and p50 values (a post-hoc descriptive addendum, no interval) and `hosts.csv` the CPU model, AVX-512F flag, selected logical CPUs, SMT sibling lists and host of each budget's shard, with the AVX-512F CPUID result and OpenCV feature line that the Rust arm's startup `cpu_dispatch` log records (empty when no completed Rust run logged one). `math-lines.csv` quotes every line of a successful Indra first attempt that holds a `$`-delimited span, the markup of Indra's math recognizer; the report sets each beside its case's strict errors and, for the laboratory crops, checks the hash-verified frozen reference for a `$`. It is quoted, never scored. Main whole-workload timing stays separate from per-cohort quality. Synthetic visual smoke tests are opt-in with `OCR_REPORT_SYNTHETIC_SMOKE=/new/output/path`; their figures are labelled SYNTHETIC and are never report inputs. Rendering binds every source/data/figure hash and resets visual approval to pending.

`cases.csv` directly exports each admitted first-attempt metric and its declared tokenizer/eligibility, outcome and response/input/reference identities. `summaries.csv` retains all six Rust summary axes and CER/collapsed CER/WER denominators, coverage, failures, metric limits and micro/macro rates. The compact language/appearance table uses prespecified fastPSM3/Python-math/Rust-math columns, strict beside collapsed CER; full exports retain every arm.

`analysis.py` reads the exported CSVs back for three post-hoc descriptive views: the modern legal pages with strict, collapsed CER and WER and the share of strict errors that collapsing removes, the corpus plan's prespecified reference-length bins (split into legal and other pages where a bin holds both), and the typography, layout and condition traits the frozen manifest records. Its case-level aggregation must reproduce every exported domain summary or the render stops. `test_analysis.py` runs without Seaborn.

Latency summaries use linear type7 empirical quantiles of all admitted successful non-warmup attempts. These descriptive observations are distinct from the continuous pilot's nearest-rank quantiles.

The default bundled Bootstrap theme uses system fonts. Static Pandas tables emit escaped HTML through Markdown rather than a widget MIME type, avoiding Quarto's unused CDN widget dependencies. `receipt.py` rejects automatic HTML/CSS assets that remain unembedded; citation/download links remain ordinary links. `comparison-statuses.csv` retains every one of the 60 planned pairs, including skipped/failed/unsubmitted identities, reasons, command exit and artifact identities.

`resources.csv` reports existing startup/readiness and scoped parent CPU/charged-memory snapshots. CPU windows distinguish pre-warmup→post-workload from admission→captured driver-end (before sentinel removal/parent retirement). Memory peaks remain lifetime charged peaks including setup/startup; none are subtracted or described as summed RSS. Every memory.events key is retained, including nonzero events, without a new exclusion threshold. The strict loader binds sibling driver records, exact budget identities, canonical counters, nondecreasing cumulative values and readiness arithmetic before exposing a row. No new OCR experiment is run.
