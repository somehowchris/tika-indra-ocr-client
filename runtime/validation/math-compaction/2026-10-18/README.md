# Native same-age compaction correctness gate

The feature-only source `ebc64f94a8bb5d9e2eb88dbec217dead3a58532c`
passed both one- and two-CPU native gates using the pinned recovered native
archives. This is a local diagnostic build, not a signed release or a fresh
native archive build. The earlier prospective protocol commit was `b259a7b`;
review amendment `35172dd` separated bit verification from timing before any
candidate predictions. Rust 1.99.1 strict Clippy passed for all new targets.

Each compact arm matched 25,243,936 mapped float elements against an independent
ordinary stream at finite `atol=rtol=1e-5`. Both input and output self caches,
both cross caches, masks and logits were included. Every ordinary token/done
snapshot matched the actual production trace at every original row/step. Token
history, stop reason/step and complete accepted UTF-8 for every original row
matched the feature-only actual production output snapshots. The ordinary
control gates independently passed 66,625,920 mapped float comparisons/profile.

| Observed metric | Ordinary | Compact |
|---|---:|---:|
| Actual decoder row-steps | 360 | 158 |
| Actual live sizes | 6 | 6 → 4 → 3 → 2 → 1 |
| Input tensor views constructed | 124 | 288 |
| Cross K/V copied bytes | 0 | 1,996,800 |
| Self K/V copied bytes | 0 | 714,240 |
| Mask copied bytes | 0 | 1,200 |
| Tokens copied bytes | 0 | 48 |
| Moved elements immediately bit-verified | 0 | 678,066 |

Total copied bytes were 2,712,288 in each compact gate. Original-row stopping
steps were `[8,32,8,27,60,23]`. All four gates exited zero with no OOM or memory
limit events. They ran on cores 12 and 12,13 of one cache domain with exact
1/2-CPU quotas, 3 GiB memory and zero swap. These are correctness gates under
concurrent unrelated compilation; their `timing_valid=false` values do not
establish latency, throughput, memory efficiency or improvement on complete
pages/Tika. Paired timing on the study's cores 0 and 0,1 remains pending until
shared build/measurement resources are quiet.

The standalone System-allocator test performed 4,000 gathers on a prepared reused
arena: exactly zero Rust allocations, reallocations, deallocations and associated
bytes. This excludes ORT/native outputs, session/input setup and diagnostic
metadata. Its source was `35172dd`; the gather core matches the gate source.
Independent immutable-coordinate sentinel tests passed separately, including
repeated compaction, layers/heads/time coordinates, invalid selections, bounds,
zero-length caches and signed-zero/NaN bit preservation. Criterion is built but
has not been run during concurrent compilation.

Raw gate reports/stderr, cgroup limits, exit states and production UTF-8 are
included here. `reference-tensors.sha256` binds every actual production tensor.
The full original tensors, source tar, model/native identities, failed and
successful compiler logs, immutable diagnostic image ID and complete resource
snapshots remain in `/vol/md1/tika-indra-artifacts/math-compaction-study-v2` and
`/vol/md1/tika-indra-artifacts/math-compaction-builds`. The default fourteen-case
source-reference gate remains a separate requirement; this experiment has not
changed its settings or claimed its own synthetic results establish OCR quality.
