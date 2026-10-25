# Opt-in compiled Cargo Chef layer

The [archived layout prototype](CHEF.md) established metadata layout only.
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
only actual byte/static/material/lifecycle gates over every exported executable
can admit equivalence or useful reuse. No performance benefit is assumed.

The key binds both recipes, actual manifests/locks, external shared source,
runtime Cargo configuration/toolchain, native SDK recipe/content, pinned Chef and
helper bytes, target/profile/features/flags and paths. Native archives changing
at an identical path must change the SDK content identity and invalidate this
layer; stale or tampered restored bytes fail full inventory checks. A separate
small native-archive fixture must demonstrate that property with real compilation.

This implementation stays opt-in: no reviewed lock selects it. In signed
validation release run [39810108196](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39810108196) the producer and
the fresh-runner job passed its contracts, real cook, fresh hosted transport,
native-archive fixture, byte equivalence of the seven executable roles recorded
at that source, and process gates.
Compiled cache artifacts are build inputs, not the
six production candidate images. Their distribution/signature receipts remain
separate from the existing six-role release schema.


[Pinned upstream cleanup](https://raw.githubusercontent.com/LukeMathWalker/cargo-chef/v0.1.78/src/skeleton/mod.rs)
removes dummy local libraries/build scripts, but does not remove every dummy
binary/test/example. This experiment therefore runs package-scoped Cargo clean
for every local normalized runtime workspace package before exporting target
state. Registry crates and the real external shared dependency remain reusable.
The required tiny fixture uses package version0.0.1 and old source mtimes so
normalization or timestamp luck cannot excuse a stale dummy executable. Actual
byte and runtime gates for every exported executable remain mandatory.

The optional `run_sdk_proof` input in the existing Signed release workflow is
false by default and accepted only on a `v*` tag. It runs after all six original
production images pass their unchanged gates and are signed. The producer uses
the same default builder, exports and reads every SDK/Chef OCI blob, publishes
unique source/run/attempt references, and signs separate image, SPDX and build
input predicates with the existing release workflow identity. No mutable branch
trust or six-image schema changes are introduced.

A dependent read-only GitHub-hosted job rejects any initial builder cache record
before downloading dependencies. It verifies exact certificates, source/ref/run,
predicates, digests, full SDK/Chef contents, and the hash of every executable
that the engine-acceptance and server-acceptance targets export (eleven roles)
against the actual source-built baseline. It retains full static checks and
material inventories, and runs the real engine and server lifecycle suites at
1 and 2 CPU. Baseline artifact transport uses a deterministic inner tar with safe
bounded extraction to preserve original executable and source modes through
GitHub's artifact service. Raw registry OCI inputs remain retained by immutable
digest; small signed manifests, logs and receipts remain in Actions artifacts.

Five negative admission cases change actual external shared source, lock bytes,
features, release profile, or expected SDK content identity. Each must fail at
its own identity gate, not with an arbitrary compiler/transport error: the four
source-side cases at the cooked dependency key check in `chef-restored`, the SDK
content case at SDK admission in `native-build`. A diagnostic from any other
gate fails the proof. In release run 39810108196 each case failed at its gate.
A separate real tiny C/Rust diagnostic retains an old compiled target, changes
`/native-fixture/libnative_answer.a` from return1 to return2, checks that
`verify` rejects the old target under a fixture-local key that embeds the archive
SHA-256, and compares warm relink output with a clean return2 build. That key is
not the production Chef key, so the rejection shows only that `verify` refuses a
differing key; the production key binds native content through
`sdk_content_sha256`, which `test_chef_cook.py` checks. Its source version0.0.1 and mtime1 also exercise the
upstream dummy executable hazard. This small archive experiment is explicitly
separate from production ONNX SDK byte equivalence.

After correctness passes, a frozen schedule executes three pairs each for warm,
actual runtime source comment change, and material documentation change. Pair
order alternates SDK/Chef, Chef/SDK, SDK/Chef. Both arms consume the same verified
SDK; the SDK arm compiles dependencies from actual source and the Chef arm
restores compiled dependencies. Each timed observation includes both original
engine/server build targets and local export. Every cell must preserve every
executable role hash. Retained logs show BuildKit vertices, direct client accounting
and complete whole-build wall time. Results report min/median/max and paired
seconds; n=3 does not support tail latency claims. Initial download/materialize
and producer cook/export/signing are retained separately and must be included in
any amortized cost analysis. This compares Chef with the SDK source consumer;
it does not execute or replace the separately frozen historical WP1 comparison.

The cost schedule preserves the original Cargo commands and their jobs=2 setting.
`BUILD_CPUSET=0,1` applies to the native source-build command; it does not admit
CPU affinity or quota for those Rust compiler stages. Runner/controller CPU and
Docker daemon capacity are retained as context, without claiming a compiler
resource cap. These build-cost observations are separate from the actual 1- and
2-CPU runtime acceptance gates.

In run 39810108196 the cook, the fresh hosted proof and the paired cost schedule
completed. Its descriptive results are not an improvement claim; choosing Chef or
SDK still needs the reviewed net-cost decision. Compiler parallelism, ccache and sccache
remain unmeasured alternatives rather than accepted or rejected optimizations.
The first source-built SDK can still incur the measured historical native build
cost; durable transport amortizes that cost only after fresh restore succeeds.
