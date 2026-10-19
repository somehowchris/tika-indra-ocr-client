Both CPU-cap correctness gates passed with the reviewed static driver. Each cap ran
one separate reference and the off/enabled arms; all 48 scheduled requests matched
their same-cap golden response. Every actual role and dependency hash matched.
Enabled arms delivered the 12-request gateway/Java/native graph, named counters and
one actual classified Java-failure Sentry event after the successful cohort.
Off arms received no exports. All six containers exited 0 without OOM, reaped owned
children and were removed; controller/cleanup exited 0.

These CPU 12 and 12,13 runs establish implementation correctness only. They do not
measure SDK overhead, establish representative OCR quality or replace the signed
source-release gates. `summary.json` preserves exact identities, raw inventory,
privacy scope and the distinction between SDK context IDs and actual OTLP edges.

The two earlier failures remain failures. The actual envelope identified a
synthetic-asserter mismatch with pinned Sentry 0.49.3 wire serialization; see
`runtime/validation/telemetry-http-sentry-wire.md` and its actual-envelope regression.
No workload, tolerance, CPU budget or successful-cohort boundary changed.
