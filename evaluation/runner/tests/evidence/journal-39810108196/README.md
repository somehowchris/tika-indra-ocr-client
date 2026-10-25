# Sealed journal excerpt

[`requests-excerpt.jsonl`](requests-excerpt.jsonl) holds eight records copied
byte for byte from two `requests.jsonl` journals in the combined artifact
`ocr-evaluation-92ba8d549e32f6a12b9e6154b238114cf754c10b-39810108196-1-combined`
of release run
[39810108196](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39810108196).
The journal SHA-256s are the ones in that artifact's `SHA256SUMS`.

| Excerpt lines | Journal lines | Journal under `shards/cpu2/ocr-evaluation-results/runs/main/cpu2/` | Journal SHA-256 |
| --- | --- | --- | --- |
| 1–5 | 1, 2, 3, 4, 130 | `rust-math-b64/driver/run/requests.jsonl` | `ee2d5cebb8fc3ab34fc0a31dc51aca5827a0850fe0d28ec098c5127b7e3afb20` |
| 6–8 | 1, 4, 130 | `tesseract-best-psm6/driver/run/requests.jsonl` | `42a8364e8fbb5bf32575ddf5c895d0d91f7d04ff7c44edcb0bbd4994476e42ae` |

Together they cover every record kind, budgets with one and two peers, and
attempts with and without a writer cluster and Tesseract settings. The test
in `src/journal.rs` parses each record into the record types that the run
writer and the scorer share, writes it back through the run writer and
requires these exact bytes.
