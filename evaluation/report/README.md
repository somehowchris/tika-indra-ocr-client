# OCR report

`report.qmd` preserves the reporting source. `baseline.py` verifies and renders
the retained historical signed-container summaries with Seaborn. Its serving and
scoring counterparts are Rust; reporting Python is isolated from those binaries.

The current notebook is a **historical baseline and frozen-study explanation**.
New multilingual native measurements, their strict artifact loader, paired
comparison figures, a hashed Jupyter dependency lock and final rendered HTML are
tracked implementation work. Their absence must not be mistaken for measured
zero scores or a completed scientific report.

Run the existing standalone report checks from the repository root:

```sh
python3 evaluation/report/baseline.py
```

The producing environment used Python 3.12.14, Seaborn 0.13.2, pandas 2.2.3,
NumPy 2.3.5 and Pillow 12.3.0. Seaborn uses Matplotlib internally; this report does
not import or call Matplotlib plotting APIs. The next rendering gate must pin
Quarto and the complete Jupyter/report dependency closure with hashes, then run:

```sh
quarto render evaluation/report/report.qmd --to html
```

Every baseline row reconciles to a retained JSON source and SHA-256. The plot
checks map each bar to its policy, CPU cap, engine, x-position, color and value;
both numeric axes start at zero. The PNGs were inspected visually. Re-rendering
resets the generated visual-review field to `pending`; a new visual review must
bind its receipt to the newly rendered file hashes. Standalone chart inspection
does not substitute for complete notebook HTML inspection.

The old mostly English synthetic data must stay separate from the frozen main
and laboratory cohorts. A final report must retain all planned, failed and
unexecuted cells, preserve denominator and cluster identities, and admit only
verified score/performance artifacts. See [the results ledger](../../docs/results.md).
