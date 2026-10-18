# Opt-in compiled Cargo Chef layer

The existing `chef-layout.py` receipt remains a metadata-only historical proof.
The separate `render-chef-consumer.py` now emits actual dependency compilation or
an immutable cooked-layer consumer. Neither changes `docker/native.Dockerfile`.
Both start from mandatory, content-verified native SDK restoration.

The producer installs checksum-locked Chef0.1.78, prepares root/runtime recipes,
uses the root skeleton for metadata only, restores the original owning root
manifest/lock and real external shared crate, and runs `cook --release --tests
--examples` for engine/server at `/app/runtime`. It sources the original native
link environment and selects the already-admitted Rust compiler. No dummy source
enters application compilation. A scratch artifact contains compiled target,
registry/git inputs, the exact recipe key, complete content inventory and cook
log. Cache mounts are not used or assumed to survive runners.

The separate consumer imports that immutable artifact, verifies its expected
content/key and recomputes the recipe against the checked-out source. Original
engine/server build stages, all four compile commands, selectors, static checks
and source/material collector remain unchanged. These commands still run after
actual source is copied. Cooking both packages may unify dependency features;
only actual seven-role byte/static/material/lifecycle gates can admit equivalence
or useful reuse. No performance benefit is assumed.

The key binds both recipes, actual manifests/locks, external shared source,
runtime Cargo configuration/toolchain, native SDK recipe/content, pinned Chef and
helper bytes, target/profile/features/flags and paths. Native archives changing
at an identical path must change the SDK content identity and invalidate this
layer; stale or tampered restored bytes fail full inventory checks. A separate
small native-archive fixture must demonstrate that property with real compilation.

This implementation is source-only and opt-in until its contracts, real cook,
fresh hosted transport, native-archive fixture, seven-role equivalence and process
gates execute successfully. Compiled cache artifacts are build inputs, not the
six production candidate images. Their distribution/signature receipts remain
separate from the existing six-role release schema.


[Pinned upstream cleanup](https://raw.githubusercontent.com/LukeMathWalker/cargo-chef/v0.1.78/src/skeleton/mod.rs)
removes dummy local libraries/build scripts, but does not remove every dummy
binary/test/example. This experiment therefore runs package-scoped Cargo clean
for every local normalized runtime workspace package before exporting target
state. Registry crates and the real external shared dependency remain reusable.
The required tiny fixture uses package version0.0.1 and old source mtimes so
normalization or timestamp luck cannot excuse a stale dummy executable. Actual
seven-role byte and runtime gates remain mandatory.
