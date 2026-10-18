# Controlled native build-cost investigation

The supplied optimization patch and proof PDF/ZIP are not available here. No
candidate speedup or executable equivalence has been established. This freezes
[the comparison protocol](protocol.json), preserves actual historical logs and
provides audit tooling before any optimization experiment. An independently
implemented candidate must be identified separately from the supplied patch.

The historical source `4bae531b8d33b18017eeb04c03a1b04bd752dc02` and integration
source `2193324` differ in application source closure, static-check enforcement,
profiling stages and native stage structure. Compare each candidate recipe with
its own unchanged source baseline. A direct cross-revision elapsed comparison
cannot attribute improvement to the proposed Docker refactor.

Actual retained GitHub data establishes:

| Run | Job wall time | Native build vertex |
| --- | --- | --- |
| [39114744717](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39114744717) | 86m12s | 4416.3s |
| [39115648687](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39115648687) | 11m56s | CACHED |
| [39143910423](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143910423) | 10m09s | CACHED |
| [39143879302](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143879302/job/117723659565) | 83m21s | 4436.7s after missing cache blob |
| [39143879263](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143879263/job/117723659437) | 54m59s | 2888.5s after missing cache blob |

The cold log also records exporting layer
`sha256:0b80f250d5eed89a9dc20de7eaa124aa5e62ce4624adfe71db076a273dd5ad82`
in 91.3 seconds. Two subsequently identified runs at the exact historical source
`4bae531b8d33b18017eeb04c03a1b04bd752dc02` independently establish the reported
miss. Both logs first import a GHA cache manifest successfully, then report that
same blob as missing and execute native compilation. Service run `39143879302`
records the missing blob at lines 768/771 and `#12 DONE 4436.7s` at line 4126.
Its build step lasts exactly 82m40s; native vertex elapsed is 89.4496% of that
step. The whole job lasts 83m21s. These are different denominators. The native
acceptance job shows the same blob failure at lines 778/781, followed by
2888.5s native execution. This establishes the missing-blob failure and rebuild;
it does not establish whether eviction, scope overwrite or another cache-storage
condition caused the missing blob. The different native elapsed times are not a
controlled hardware comparison.

[The matched receipts](evidence/matched-4bae/summary.json) retain original API
metadata, original log SHA256 values, raw ZIP SHA256/lengths, deterministic gzip
log copies and reader outputs with every failed/cached/completed terminal update.
The original downloaded ZIPs remain in the named local evidence directory.
The 7960bc-prefixed historical SDK identity remains unverified. Cached vertices
have no measured execution duration. BuildKit vertex elapsed costs overlap;
never sum them to claim job wall time.

```sh
python3 -B evaluation/build-cost/test_audit.py
python3 -B -O evaluation/build-cost/test_audit.py
python3 -B evaluation/build-cost/audit.py read-log JOB.log --output cost.json
python3 -B evaluation/build-cost/audit.py record-phase --phase vendor \
  --output NEW_ATTEMPT_DIRECTORY -- cargo vendor --locked
```

The phase recorder retains actual exit status, stdout/stderr hashes, monotonic
elapsed time and `wait4` child CPU/maxRSS. It cannot attest resources consumed by
an external BuildKit daemon. Instrument separate actual builder-side commands for
native configure/compile/install, Rust builds, vendor, both material archives,
OCI export, cache export and push. Historical combined `RUN` costs cannot be split
retrospectively. Preserve failed and cancelled attempts. The protocol requires
five paired repetitions and a frozen alternating order before performance claims.

`compare-pair` requires both seven-role export directories, both extracted native
SDK directories and both extracted materials directories. It records exact
binary hashes and real `readelf` output, rejects dynamic executables, missing
roles, symlinks and nonregular inventory entries, and compares path/type/mode/
size/hash inventories. Output must be external to all inputs. Any failure is
retained in an exclusive attempt directory. It establishes only this content
gate; lifecycle, source provenance, archive identities and fresh-builder SDK
materialization remain separate required gates.

Durable SDK admission comes first. Recipe identity includes exact base digest,
Debian snapshot, native lock/scripts/toolchain/compiler flags and source/download
closure. Content identity binds actual static archives, headers and Cargo link
environment. OCI identity binds the selected platform manifest/config/blobs.
A fresh builder must restore immutable SDK content, fail closed on a missing or
corrupted blob and link all seven roles without native recompilation. This is
separate from whether an ephemeral GHA cache says `CACHED`.

Cargo chef is a separate experiment. The verified upstream release is
[v0.1.78](https://github.com/LukeMathWalker/cargo-chef/releases/tag/v0.1.78),
GNU archive SHA256 `70ef940ef90d04d122f0176fdb8d6c39069191b484a1eaa29b327370c2e1c3c0`.
Actual prepare on immutable integration source succeeds for root and runtime
workspaces. The root recipe includes `shared/trace-context`; the runtime recipe
omits that external sibling dependency. Rehydrating the runtime manifests alone
fails actual offline `cargo metadata` with missing shared manifest, exit101.
The two recipes and failure are retained. No cook or compiled-cache benefit has
been demonstrated. An initial prepare run from the wrong review worktree is
retained locally as unadmitted and excluded from these receipts.

Preserve the original `/app/Cargo.toml`, root-owned shared crate and `/app/runtime`
layout before attempting cook. Keep original compiler/profile/native flags,
features and all original test/build commands; verify all seven byte identities
and lifecycle gates. The [upstream chef instructions](https://github.com/LukeMathWalker/cargo-chef#limitations-and-caveats)
require cook/build in the same directory with the same toolchain.
[Docker's GHA cache documentation](https://docs.docker.com/build/cache/backends/gha/#scope)
explains scope overwrite/access constraints; its
[cache-mount documentation](https://docs.docker.com/build/ci/github-actions/cache/#cache-mounts)
states mounts are not preserved in GHA cache by default. Prefer an explicitly
transported compiled dependency layer bound to exact SDK content, toolchain,
profile and flags. Never infer cross-runner mount persistence or expand signing
trust to adopt a cache.

An [independent two-file candidate](evidence/independent-candidate/prospective.json)
now implements exact Cargo JSON selection and the materials leaf. It preserves
the native SDK stage bytes and all four original Cargo compile commands, adding
only JSON reporting. The seven roles use exact versioned local package IDs,
target names/kinds and test profiles; duplicate/missing artifacts, failed or
truncated streams, unexpected paths and overwrite attempts fail. Receipts bind
the input stream and copied executable bytes. Docker's static graph check passes;
this does not execute compilation. Run its separate contract checks with:

```sh
python3 -B evaluation/build-cost/test_selector.py
python3 -B evaluation/build-cost/test_candidate.py
```

The candidate has no speedup or native-equivalence result yet. Real Cargo JSON for all seven roles now passes the strict selector contract;
actual paired executable hashes, lifecycle gates and fresh-builder SDK restoration
remain pending. Relink source archives necessarily contain the
declared Docker recipe/selector source delta. Preserve all baseline material
paths and verify unchanged extracted contents outside that exact prospective
source delta; record both archive hashes. The strict `compare-pair` material gate
will report these differences for review, rather than silently accepting them.

The separate [local immutable SDK experiment](SDK.md) now supplies a scratch
closure producer, mandatory immutable-context consumer and distinct
recipe/content/OCI admission tooling. [Actual native packaging and admission](evidence/native-sdk-2026-10-18/summary.json)
have succeeded: 400 materialized files (285,990,279 bytes), 557 closure entries,
all 31 required notice source paths, all referenced OCI blob hashes/diff-id and
complete file modes/hashes. The native vertex was reused from cache; the shared
builder restored the SDK into the pinned toolchain and passed admission. Source
transport mode qualification and two unsuccessful development attempts remain
explicitly retained. A local admitted GNU linker image is available; no image
was published. Fresh-builder linking, original seven-role byte/static/lifecycle
gates and controlled cost pairs remain pending. The earlier tiny BuildKit OCI
round-trip remains explicitly synthetic transport validation.
