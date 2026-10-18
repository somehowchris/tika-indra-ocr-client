# Batch arithmetic verification

`BatchPlan` accounts for the aligned tensor width of an already selected native
recognition batch. It accepts a crop only when both the row ceiling and actual
scratch capacity fit. Failure preserves the prior account. See [the production
implementation](../runtime/crates/indra-core/src/batch.rs) and its use in
[`tensor_widths`](../runtime/crates/indra-engine/src/batching.rs).

## What the proof checks

The Creusot contracts cover construction, eight-column alignment (including
overflow rejection), exact maximum aligned width, successful count growth, preservation on failure,
clearing, and the invariant `count × padded_width ≤ max_work`. They also check
that the count stays within its independent limit and an empty plan has zero
width. Existing ordinary boundary tests remain in the projected source.
The strengthened `try_add` contract also states acceptance if and only if the
input width is valid and the resulting count and aligned work fit. It specifies
`InvalidWidth` precedence, including when a batch is already full. A separate
ordinary test uses a wider integer oracle for exact width, acceptance, failure
preservation and the count-overflow boundary. These strengthened conditions
require a fresh solver run; the recorded historical result below covers the
earlier, narrower contracts.

This is a proof of the bounded arithmetic component. It does not establish OCR
accuracy, native-library correctness, HTTP cancellation, concurrent lifecycle
correctness, or performance. Those require their own tests and measurements.

## Where the arithmetic guards production

The native `Plan` still selects and sorts rows with the pinned reference's raw
width ratio and area rules. `fill_batch` passes the selected crop widths and
actual float arena capacity to a temporary `BatchPlan`; its width and work
determine the tensor slice. This replaces the previous maximum-width variable
without retaining a second scheduler or duplicate state. Geometry, membership,
order, normalization and interpolation are unchanged.

Using the actual scratch bound matters. Sixty-two rows of width 657 use 40,734
raw columns and 41,168 aligned columns. Sixty-three rows of width 649 use 40,887
raw columns and 41,328 aligned columns. Both groups fit the 41,472-column text
arena; applying an aligned 40,960 limit would change reference grouping. For
multi-row text groups, raw work is at most 40,960, and padding adds at most
`7 × 64` columns: 41,408 is below 41,472. A singleton's width is at most 4,096.
The math arena has `64 × 4,096` columns. Aligning each width before taking the
maximum equals aligning the maximum width, because eight-column rounding is
monotone.

The focused native gate compares all prepared float bits, width metadata,
selected indices and untouched tensor tails against the previous packing
procedure. It covers both text boundary groups and out-of-page math crops.
Criterion compares the previous scan and production account on eight fixed
shapes. The standalone allocation gate demands zero Rust allocator churn in
both scans. These are separate integration and measurement gates; the arithmetic
proof alone does not prove tensor equality or a performance improvement.

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

## Historical verified run

[Run 39090447398](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39090447398)
completed successfully on 18 October 2026 at commit
`82625aab6d58b089e65e13b8c5d0835be3f4fb65`. Four ordinary tests passed, and the
uncached Creusot command reported **`Proved (16 files)`**. The
complete evidence archive is the `batch-arithmetic-proof` artifact
**12122671844** on that exact Actions run. It preserves the solver log,
all generated sessions and source/tool identities. Its SHA-256 is
`6be505f9ce71411724c2e02122d798bc76465d25f0fa3d3ee1b989f33f58ba03`;
it is the unmodified Actions artifact, 52,412 bytes.

The production source and inverse-projected source both hash to
`1c72e0d2570c00e71afe9b723df2677909c91503f8a0c3adf40af5ef28f947c9`.
The verified projection hashes to
`021b32e8ba01a94522fd80e892851a5c4d49460686f8ca9bc5d799bc17826167`.
These are evidence for the earlier bounded-state contracts at that exact source
revision. That run did not verify the newly strengthened exact-width and
acceptance-iff postconditions. The current projection retains the same
production-source hash and requires new uncached solver evidence before those
postconditions can be reported as proved.

Creusot's scope and trusted toolchain are described by the
[project](https://creusot.rs/) and its
[pinned source](https://github.com/creusot-rs/creusot/tree/65349025c67e69fcd7479f1f420fa5904f69bc27).
