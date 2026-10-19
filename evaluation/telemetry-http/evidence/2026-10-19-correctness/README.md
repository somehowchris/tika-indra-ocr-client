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

`raw-complete.tar.gz` retains all original protobuf/Sentry bodies, the derived
span inventories, request goldens/outcomes, logs, controls and both earlier failed
runs. It adds the bounded payloads omitted from the first compact summary.
Exporter HTTP headers and raw HTTP client wire were not retained by the original
producer; their runtime checks cannot be replayed as missing bytes. The packet
makes that limit explicit. The original SDK Sentry context also does not prove
correlation with the terminal OTLP failure; final signed-role validation is separate.

The Rust verifier is a separate build identified in `offline-verifier.json`.
It reuses byte-identical d4 wire/privacy/graph decoder modules, validates all
recorded role/dependency and response identities, and starts no model, runtime,
receiver, child process or cgroup operation. It did not produce the old d4 runs.

Run `bash scripts/replay-telemetry-http.sh STATIC_VERIFIER THIS_PACKET NEW_OUTPUT`.
The script verifies every packet checksum, extracts the complete original data,
checks all original raw manifests and terminal cleanup, then replays six cells
through the exact existing Rust wire verifier. The verifier build/ELF/test receipts
are included; executable bytes remain outside Git, bound by their SHA-256.

The actual Rust CLI also rejects five mutated copies: a wrong response digest,
a duplicate recorded asset, a private Sentry canary, an arbitrary context field
and a broken scheduled parent edge. `negative-replay.py` reproduces those test-only
mutations after positive extraction; it invokes the Rust verifier and never OCR.
The producer's raw bytes remain unchanged.
