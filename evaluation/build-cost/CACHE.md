# Cache ownership and runner boundaries

`cache-plan.py` gives each build purpose one distinct v2 writer: native archives,
engine, server, profiling, stack native/gateway, release native/gateway acceptance,
and each published release role. Application/acceptance exports no longer replace
the native archive writer's scope. Every purpose imports its own scope plus its
explicit dependency scopes. Existing v1 scopes remain read-only migration imports;
no workflow writes them. Release image scopes are derived from the validated role,
not supplied as an arbitrary caller string.

The helper admits its exact owning workflow path, repository, supported event and
ref. Same-repository pull requests write only their GitHub-isolated PR ref cache;
fork pull requests receive an empty `cache-to`. `pull_request_target` is rejected.
Push/dispatch plans retain their actual branch or tag ref. None of this changes
release certificate identities, signing triggers or artifact admission.

[Docker documents scope overwrite and branch access](https://docs.docker.com/build/cache/backends/gha/#scope):
GitHub still restricts cache visibility to the current, base and default branches.
A distinct scope does not remove those restrictions or make blobs durable. A plan
records requested imports and exports; it never asserts that a cache exists, is
complete, or was restored on a fresh runner. The matched historical missing-blob
job is retained separately. This policy does not diagnose the remote storage
cause of that failure.

Durable native SDK distribution remains a separate immutable-content operation.
A fresh-runner experiment must retain actual import/download/materialization logs,
all hashes, source/SDK identities and all seven executable/static/lifecycle gates.
No fresh-runner restoration or speedup is admitted by these source-only policy
checks. Cache mounts are not assumed to survive runners; Cargo Chef compiled
layers must bind the exact SDK, compiler, profile, flags and workspace closure.

`test_cache_plan.py` checks writer uniqueness, explicit read-only legacy imports,
fork/same-repository/ref/event boundaries, unknown fresh-runner status and all
workflow/composite cache consumers. Existing CI runs the build-tool contracts in
normal and optimized Python modes before Rust/native work. These tests contact no
cache backend and execute no builds.
