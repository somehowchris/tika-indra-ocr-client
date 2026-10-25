# Independent completed-profile admission

Run `scripts/admit-native-profile.py` only after the controller has completed and
other measurements have stopped. It refuses incomplete runs before hashing or
sample analysis. Supply independently recorded full source, tools image,
protocol and bundle-manifest identities; do not derive expected values from an
unreviewed result and call that provenance proof.

The read-only report requires the full 1/2-CPU four-phase protocol, all eight
successful phase/resource captures, exact host PID/container ownership, measured
CPU affinity/CFS/memory/swap/task/stack limits and no OOM. It checks private-copy
inventory, unchanged host owner/mode, and exact container/volume cleanup receipts.
It rechecks all locked model file bytes, the source/profile/protocol bindings,
complete original manifest-verification receipts and all command exit records.

Each CPU cap must contain every prospective Criterion case identity the
protocol lists, exactly `criterion.expected_total_cases` of them (77 since
[amendment 0003](../runtime/validation/audit/native-profile-amendment-0003.json),
46 before), with 100 finite positive samples and valid estimates per case. Four
captures must replay through the original frozen sample validator, with zero
lost events, matching process IDs and exact folded/raw sample counts. All three
64-request blocks retain fixed OCR settings and exact expected fixture output.

A successful report admits these correctness/resource/sample receipts. Inspect
all four flamegraphs and representative raw symbol stacks separately before
marking visual review complete. Interpret sampled unknown frames explicitly.
Retain partial or failed runs and all their logs. These diagnostics establish no
Tika throughput, population latency, OCR quality, cold-start or causal speedup.


The independent expected affinity defaults to CPU0 for the one-CPU cap and
CPU0,CPU1 for the two-CPU cap. Captured expected/effective/process CPU lists must
match those exact lists, not only their cardinality. Explicit alternative
`--expected-cpus-1`/`--expected-cpus-2` values require an independently frozen
study protocol.

Provide `--activity-note` with the actual operator disclosure. For the
2026-10-18 baseline v2 run, a short semantic code search ran around 22:35–22:36 UTC
during the two-CPU Criterion phase. Existing host workloads were also present.
Retain the available window process/load evidence and interpret the results as
descriptive. Do not claim an otherwise idle host or causally isolated speedup.
