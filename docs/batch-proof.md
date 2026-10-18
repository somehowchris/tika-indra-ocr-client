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

The CI solver step fails on undischarged goals. Artifacts preserve source and
projection hashes, tool identities, complete solver output and generated proof
sessions on success or failure. Compilation alone is not accepted as a proof.
The recovered `evidence/recovery.json` describes an earlier compile-only attempt;
it is historical evidence, not the current solver result.

Creusot's scope and trusted toolchain are described by the
[project](https://creusot.rs/) and its
[pinned source](https://github.com/creusot-rs/creusot/tree/65349025c67e69fcd7479f1f420fa5904f69bc27).
