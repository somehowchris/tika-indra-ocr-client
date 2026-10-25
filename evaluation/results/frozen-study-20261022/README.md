# Frozen multilingual study: sealed tables and index

Byte copies of the exported tables and the combined study index of the frozen
24-cell study, measured by release run
[39810108196](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39810108196)
on source `92ba8d549e32f6a12b9e6154b238114cf754c10b` and rendered from that
sealed evidence by run
[39837656304](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39837656304)
with report code `3ef7e0883b858420520192d8d5efb4bf9aa5e0a5`. Both runs'
artifacts expire on 2027-01-20; these files keep the numbers in the
[results ledger](../../../docs/results.md#measured-result-release-run-39810108196)
and the README traceable after that. No file was edited.

| Files | Artifact | Path in the artifact |
|---|---|---|
| `study-index.json` | `ocr-evaluation-92ba8d549e32f6a12b9e6154b238114cf754c10b-39810108196-1-combined`, id 12450136764, zip `sha256:0592d4438b8fd25b82d75673cc57663bba2ce1825bae2a8f1bbda4e59eb3b3da` | `study-index.json` |
| `attempts.csv`, `cases.csv`, `comparison-statuses.csv`, `paired.csv`, `performance.csv`, `quality.csv`, `resources.csv`, `summaries.csv` | `ocr-report-3ef7e0883b858420520192d8d5efb4bf9aa5e0a5-39837656304-1`, id 12450984040, zip `sha256:7035532d090e9c6056f4fab33ca0d4a109a0d7331c44018420343c6a3cc41a29` | `report/study-data/` |
| `render-receipt.json` | the same report artifact | `report/render-evidence/receipt.json` |

`SHA256SUMS` lists every data file (`sha256sum -c SHA256SUMS`). The CSVs keep
their CRLF records; `.gitattributes` stops Git from converting line endings.
[`test_frozen_study_data.py`](../../report/test_frozen_study_data.py) checks the
chain from committed receipts to these bytes:

- [`frozen-study-visual-review.json`](../../figures/frozen-study-visual-review.json)
  records the SHA-256 of `render-receipt.json` and the study index SHA-256
  `e041b21073a4be7e02dae69665bc6e33d3e5cf42426362d9d0b47b5303958d5d`.
- `render-receipt.json` binds each CSV's SHA-256 and size, the measured source
  and that index SHA-256, which is the SHA-256 of `study-index.json`.
- [`frozen-study-plot-validation.json`](../../figures/frozen-study-plot-validation.json)
  binds the same CSV hashes.

The raw per-cell evidence that `study-index.json` names by path and hash
(responses, journals, command records, scores and comparison files) is only in
the 78 MB combined artifact. A re-render or the
[independent numerical replay](../../independent/README.md) needs that
artifact; these tables alone do not allow re-scoring.
