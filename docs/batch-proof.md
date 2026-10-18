# Batch arithmetic verification

`BatchPlan` admits a crop only when both the crop-count ceiling and the final
padded tensor-width budget fit. Padding happens before admission. A rejected
crop leaves the prior plan unchanged; the worker can flush it and retry the crop
without losing work. See [the production implementation](../runtime/crates/indra-core/src/batch.rs).

## What the proof checks

The Creusot contracts cover construction, eight-column alignment (including
overflow rejection), successful count growth, preservation on failure,
clearing, and the invariant `count × padded_width ≤ max_work`. They also check
that the count stays within its independent limit and an empty plan has zero
width. Existing ordinary boundary tests remain in the projected source.
The contracts do not specify that the chosen width is exactly the maximum of
all admitted crop widths, or that every fitting crop is accepted. Those
functional properties remain covered by ordinary boundary tests; this proof's
claim is bounded, aligned state and preservation on rejection.

This is a proof of the bounded arithmetic component. It does not establish OCR
accuracy, native-library correctness, HTTP cancellation, concurrent lifecycle
correctness, or performance. Those require their own tests and measurements.

## Reproduce

The [workflow](../.github/workflows/batch-proof.yml) installs Creusot **0.14.0** at
commit `65349025c67e69fcd7479f1f420fa5904f69bc27`. Its upstream dependency manifest
pins Why3 and Why3find revisions, and its installer pins solver archives. The
proof crate pins `nightly-2026-08-03` and its Cargo dependency lock. Follow the
[release installation guide](https://guide.creusot.rs/v0.14.0/installation.html)
for the host prerequisites, then run from the repository root:

```sh
cargo +1.99.1 run --locked --manifest-path runtime/proofs/projector/Cargo.toml -- --check
cd runtime/proofs/batch-plan
cargo test --locked -- --test-threads=1
cargo creusot --no-cache --why3session -j 2
```

The Rust projector inserts the declared contracts, invariant, three checked
arithmetic bridge lemmas, and one erased proof assertion that connects the
alignment mask to those lemmas. The bridge lemmas must themselves be proved;
they split a difficult mixed bitvector/integer goal without weakening it. The
inverse must reconstruct every production-source byte, including comments and
tests. `--check` fails when the checked-in projection is stale. To intentionally
refresh it, run the same command without `--check`, inspect the diff, and rerun
the solver. No trusted proof assumptions are inserted.
The contract-aware equality derive is an explicit Creusot verification adapter
for the fieldless error enum. Exact source erasure does not claim identical
generated compiler IR between the ordinary and verification builds.

The CI solver step fails on undischarged goals. Artifacts preserve source and
projection hashes, tool identities, complete solver output and generated proof
sessions on success or failure. Compilation alone is not accepted as a proof.
The recovered `evidence/recovery.json` describes an earlier compile-only attempt;
it is historical evidence, not the current solver result.

## Verified run

[Run 39090447398](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39090447398)
completed successfully on 18 October 2026 at commit
`82625aab6d58b089e65e13b8c5d0835be3f4fb65`. Four ordinary tests passed, and the
uncached Creusot command reported **`Proved (16 files)`**. The
[complete evidence archive](evidence/batch-proof-20261018.zip) preserves the
solver log, all generated sessions and source/tool identities. Its SHA-256 is
`6be505f9ce71411724c2e02122d798bc76465d25f0fa3d3ee1b989f33f58ba03`;
it is the unmodified Actions artifact, 52,412 bytes.

The production source and inverse-projected source both hash to
`1c72e0d2570c00e71afe9b723df2677909c91503f8a0c3adf40af5ef28f947c9`.
The verified projection hashes to
`021b32e8ba01a94522fd80e892851a5c4d49460686f8ca9bc5d799bc17826167`.
These are evidence for the stated contracts at that exact source revision.

Creusot's scope and trusted toolchain are described by the
[project](https://creusot.rs/) and its
[pinned source](https://github.com/creusot-rs/creusot/tree/65349025c67e69fcd7479f1f420fa5904f69bc27).
