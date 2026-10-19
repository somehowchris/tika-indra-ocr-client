The actual Sentry 0.49.3 envelope from the controlled Java-child fault is retained as
`examples/telemetry_http/fixtures/sentry-java-exited-0.49.3.envelope` relative to the gateway crate.
Its byte identity and pinned dependency source hashes are in `telemetry-http-sentry-wire.json`.
The original failed correctness runs remain unchanged.

[`Event::level`](https://docs.rs/sentry-types/0.49.3/src/sentry_types/protocol/v7.rs.html#1639-1644)
defaults to `Error` and omits that default during serialization. Missing `level` therefore
means error; explicit null or another level is rejected. The first synthetic fixture
incorrectly required an explicit field.

[`Scope::apply_propagation_context`](https://github.com/getsentry/sentry-rust/blob/0.49.3/sentry-core/src/scope/real.rs#L466-L477)
adds an SDK trace identity when no active transaction exists. Admission permits exactly
`type: trace`, a nonzero 32-hex trace ID and a nonzero 16-hex span ID. Additional contexts
or trace fields fail. This SDK identity alone does not establish an OTLP parent edge.
The separately verified gateway/Java/native OTLP graph establishes those edges.

The accepted event still requires the exact `java_exited` class, gateway phase,
bounded message, exception and lifecycle breadcrumbs. User/request fields, stack
payloads, arbitrary context data and private canaries remain rejected. A rejected
bounded wire payload is retained before admission, receives no success response
and cannot contribute an accepted event or successful cell.

The real-envelope regression includes invalid levels, wrong classification, zero or
malformed IDs, extra trace/context fields and private request data. No source,
workload, CPU budget, tolerance or successful-request timing boundary changed.
