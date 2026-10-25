# Synthetic fixtures and bridge measurement

`results/synthetic-fixtures/` holds the deterministic synthetic pages, their reference transcripts and the case manifests that record the SHA-256 of each page and transcript. The recorded parity harnesses read `jpeg_200.jpg` and `skew_200.png` at this path. `results/bridge-streaming-2026-10-18/` records the allocation and Criterion measurement of the Rust bridge's streaming response path.

The measured packets of the Python-backend baseline study and the harness that produced them were removed from the delivery tree; [the implementation history](../docs/history.md) lists them with links to the archive commit and, where one tree or blob covers a row, its Git ID. The current scientific comparison is indexed by [the results ledger](../docs/results.md).
