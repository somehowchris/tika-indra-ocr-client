# Cargo chef workspace prototype

The pinned [cargo-chef v0.1.78](https://github.com/LukeMathWalker/cargo-chef/releases/tag/v0.1.78)
GNU archive and executable hashes live in
[`cargo-chef.lock.json`](../../scripts/cargo-chef.lock.json). The initial runtime
recipe omitted its external `shared/trace-context` dependency. A complete runtime
recipe alone cannot recreate this repository's separate owning root workspace.

[`chef-layout.py`](../../scripts/chef-layout.py) now verifies the exact Chef
executable, prepares both recipes, invokes upstream `cook --no-build` for root
metadata stubs, restores the original root manifest/lock and complete real shared
crate, then invokes runtime `cook --no-build`. Actual locked offline runtime
metadata succeeds. The original external shared package retains version0.1.0;
upstream-generated runtime dummy packages use0.0.1. Source inputs remain unchanged.
[Receipts](evidence/chef-layout-2026-10-18/admission.json) include every command.

```sh
python3 -B scripts/chef-layout.py \
  --source /absolute/immutable/source \
  --source-commit FULL_40_CHARACTER_COMMIT \
  --chef /absolute/verified/cargo-chef \
  --sdk-receipt /absolute/admitted/sdk/materialization.json \
  --output /absolute/new-layout-attempt
```

This helper never requests dependency compilation. Its prospective dependency
key binds both recipes, the original owning root manifest/lock and actual shared
source, exact SDK compiler recipe/content, Chef/helper bytes, target, profile and
Rust flags. OCI distribution timestamps are separate from compiler-content
identity. The layout lives outside the source. Root target placeholders serve
manifest discovery only; actual source replaces all dummy targets/manifests
before the four original compiler commands and seven-role acceptance gates.

The next experiment is an explicitly transported compiled dependency Docker
layer under the admitted SDK/toolchain, built from the same `/app/runtime`
working directory as the original commands. Preserve `/app` owning metadata and
`/app/shared` real code. Source `/native/cargo-env.sh`; keep target,
`RUSTFLAGS`, profile, features and compiler identities unchanged. A cook covering
engine/server tests/examples is a dependency precompile candidate; it must not
replace or weaken any original build/test command. Measure cook cost, application
rebuild cost and artifact transport separately. A cache hit on one builder is not
evidence of fresh-runner persistence.

Cook compilation, seven native-role hashes, lifecycle acceptance, fresh-runner
transport and performance benefit remain pending. No Chef dependency cache has
been built or adopted. This source/layout work ran while profiling was active;
no native/Rust compiler, OCR process or timing experiment was started.
