# Local full-stack trace correctness, 2026-10-18

Both 1-CPU and 2-CPU gates passed with the stricter source in tracing commit
`060b4fe` (including child-correlated Java parse overlap). These are diagnostic
correctness checks using the corrected canonical native build and recovered
pinned native archives. They do not establish a signed release, a fresh native
archive rebuild, representative OCR accuracy or performance comparisons.

The complete gateway, forked Tika 3.3.2 overlay, Rust bridge, native OCR, driver
and local OTLP sink ran within one Docker cgroup: 1/2 CPU quota, 3 GiB memory,
zero swap, no network, read-only root. Affinity used physical cores 8 and 9,
within the same L3 domain on this Ryzen 9 7950X3D. `limits.json` retains selected
non-secret execution fields; the daemon identity chain is recorded separately.

Each gate required exact gateway → Java parser → OCR child → native HTTP → native
inference ancestry; overlapping gateway and child-correlated Java calls; valid,
missing, malformed and duplicate contexts; unchanged two-page text; expected
unsupported-output errors; absence of private payloads; and normal shutdown with
complete orphan reaping. All SDKs finished shutdown before the first 60-second
export interval, and the total span cap is smaller than any component's full
batch trigger, proving shutdown flushing. Java build tests also passed.

Raw OTLP protobuf, decoded graph, request outcomes/hashes and actual binary/JAR
identities are retained here. The JSON latency fields are execution diagnostics.
The full local capture and failed build attempts remain under
`/vol/md1/tika-indra-artifacts/tracing-canonical-v2` and
`/vol/md1/tika-indra-artifacts/tracing-build-attempts`. Final release candidates
must rerun the mandatory gate against their own exact digests before signing.
