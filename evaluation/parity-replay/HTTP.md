# Exact production HTTP text replay

The fourteen-case replay recorded the separate native client `26214af1...` and its
850 diagnostic tensors. It does not directly prove the newer HTTP server ELF
`c9cfe911...`. Source/material closure and separate engine/server test harnesses
support that server; their different code generation remains explicit.

`examples/http_reference.rs` closes the observable HTTP output gap. It reuses the
bridge library's bounded HTTP headers and byte-preserving UTF-8 reader, preserving
U+000C page separators. It does not use the bridge CLI's XML-safe text conversion.
The static diagnostic records bounded raw wire responses, including error bodies,
and rejects incomplete, ambiguous or extra response bytes. Its hold/exec mode lets
the host attest its exact owned Docker cgroup before any model loading, without a
shell, extra capability, host PID namespace or host control socket.

`verify-native-http-reference.sh` binds the exact `cbc65a...` assembled image and
`c9cfe911...` server, actual mounted client ELF, frozen selection/comparison hashes,
all fourteen input/reference text hashes and model inventory. The actual client
producing commit/build receipt remains separate from the controller commit;
tracked client/Cargo/transport source equality is checked across them. The exact
server help and complete flag configuration are validated before model admission.
The server uses its production60s request/75s shutdown budget; the client retains
a separate420s outer deadline. It executes13
math-disabled cases and the single math-enabled case with fresh servers at both
1 and2CPU limits, for28 responses. Worker count1, queue2, batch64 and spinning-on
match the ordinary implementation; native/ORT/OpenCV threads equal the CPU budget.
Each server and its client share3GiB memory, zero swap and CPU12 or12,13. Cases
are sequential; this is correctness evidence, not a throughput experiment.

```sh
RUSTFLAGS='-Ctarget-feature=+crt-static' cargo build --locked --release \
  --target x86_64-unknown-linux-gnu --example http_reference
scripts/verify-native-http-reference.sh \
  /absolute/static-http-reference /absolute/telemetry-identity.json \
  sha256:cbc65a508d432318d98a53587f555bb60f54737327918ec49b3501f5c6feda92 \
  /absolute/models /absolute/frozen-inputs /absolute/raw-reference \
  /absolute/client-build-receipt /absolute/new-output
```

The raw reference directory is the preserved replay artifact containing
`selection-plan.json`, `comparison-protocol.json` and `reference/CASE/text.txt`.
No reference predictions, frozen protocols or old receipts are rewritten. Every
response must match the existing reference text exactly. Preserve mismatches,
request stderr, wire bodies, cgroup counters, image/client/server identities and
cleanup failures. A pass proves28 selected exact HTTP responses and ordinary
shutdown under these limits. It does not claim a new850-tensor replay, OCR quality,
signed release acceptance, telemetry graph correctness or measured performance.
