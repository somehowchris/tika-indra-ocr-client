# Cache ownership and runner boundaries

The unmeasured ownership proposal in `cache-plan.py` gives each build purpose
one distinct v2 writer: native archives,
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


## Actual storage pressure and adoption gates

The [fully paginated read-only API snapshot](evidence/cache-pressure-2026-10-18/summary.json)
at 2026-10-18 22:30:43–44 UTC contains 125 distinct cache IDs totaling
11,430,362,546 bytes (10.645 GiB). Both listing pages report 125 total entries;
the separate usage endpoint agrees with their summed bytes. The snapshot is not
atomic, because those endpoints can race writers. It retains raw API pages,
endpoint URLs, timestamps and hashes; no entry was deleted and no blob downloaded.
Of the listed bytes, 7,917,378,847 belong to PR11's merge ref, 1,861,598,686
to the recovery ref and 1,651,385,013 to PR27's merge ref. Five cache keys recur
across refs, including the same 236,958,924-byte blob key on all three refs.
Several individual entries exceed one billion bytes. These are metadata key/size
observations, not independent verification of the underlying blob content.

[GitHub documents a default cache limit that can be increased](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#usage-limits-and-eviction-policy)
and [configurable repository storage limits](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository#configuring-cache-storage-for-a-repository).
The configured limit for this repository was not captured. The observed size is
near the default 10 GB scale; it is not proof of a quota violation or the cause
of the historical missing blob. Distinct scope ownership fixes writer collisions;
it does not itself solve storage pressure. Fourteen `mode=max` writers could
increase retention/export costs and churn. Do not promote that configuration as
a measured performance fix.

[Docker's cache-mode contract](https://docs.docker.com/build/cache/backends/#cache-mode)
exports only resulting-image layers with `mode=min`, whereas `max` includes
intermediate stages. The tradeoff must be measured against the actual graph:

| Candidate | What it may save | Required qualification |
| --- | --- | --- |
| Existing shared scope, `max` | Historical comparator | Retain overwrite/missing-blob failures; never treat a cache manifest as restoration proof. |
| Distinct purpose scopes, `max` | Independent dependency/intermediate reuse | More scopes can retain more intermediate payloads across refs; measure total/export bytes and complete runner cost. |
| Distinct application/acceptance leaves, `min` | Smaller exports | Scratch leaves omit compiler/dependency stages; a source change may trigger a full Rust rebuild. Native evidence-only leaves also omit static archives. |
| Immutable compact native SDK plus selective dependency-layer export | Native closure survives ephemeral cache state | Requires existing signature/trusted-distribution admission, real fresh-runner materialization and exact seven-role/static/material/lifecycle gates. Compiled Chef dependencies remain a separate measured layer. |

Before adoption, freeze the selected writer set, export mode per target, numeric
storage/headroom budget against the actual configured limit, allowed ref matrix,
runner/compiler/SDK identities and alternating comparison schedule. Keep legacy
imports read-only and existing original cache state intact. Use separately named
experiment scopes, not production-scope replacement. Preserve failures.

Report full job wall time, actual cache import/export time and transferred bytes,
unique cache entries/bytes by scope and ref before/after, and genuine fresh-runner
restore outcomes. Include compiler/configuration, content/static and lifecycle
gates; unchanged binaries alone cannot admit a broken restoration path. Record
whether native archives or Rust dependencies were recompiled. A warm local run
cannot satisfy the fresh-runner gate. Select a policy only after the frozen cost,
storage/headroom and correctness criteria pass; no such experiment has run yet.
