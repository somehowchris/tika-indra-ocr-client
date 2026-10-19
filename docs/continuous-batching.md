# Experimental batching across two requests

The opt-in worker can put compatible lines from **two admitted requests in one
ONNX Runtime invocation**. It shares the script classifier and the ordinary or
Chinese CTC recognizer, then scatters results back by owner and original line.
A completed slot can accept the next FIFO request while its peer continues.
The ordinary worker remains the default. The frozen quality study and its eight
arms do not enable this experiment.

The real engine/server gates pass at both CPU limits: canonical two-owner
script/text/Chinese model outputs, refill while a peer remains live, cancellation
isolation and accepted work under drain/SIGTERM. These gates exercise the
unchanged engine source; they do not prove a diagnostic driver has completed its
paired study. The 240-cell native/Tika run has started; complete admitted timing
results and a deployment decision remain pending. A successful type check or a
queue containing two requests does not establish a shared native call. The [prospective protocol](../runtime/validation/continuous-protocol.json)
keeps these evidence requirements separate.

## Enable explicitly

```sh
indra-server --workers 1 --queue-capacity 2 --threads 1 --cv-threads 1 \
  --batch-size 64 --experimental-continuous-batching true serve
```

The same setting is `server.experimental_continuous_batching` in TOML and
`INDRA__SERVER__EXPERIMENTAL_CONTINUOUS_BATCHING` in the environment. Flags
override environment values, which override the selected file and defaults.
The configuration rejects this experiment with two workers. At two CPUs use one
worker with both native thread counts set to two; that is a thread setting, and
the enclosing cgroup must independently enforce the total CPU limit.

Admission capacity is still `workers + queue_capacity`, bounded by the existing
eight-slot pool. The example permits three total admitted requests, including
uploads and responses whose final bytes are still referenced. The pilot adds no
queue, timer, request byte copy, model session or async inference task.

## Ownership and execution

`Engine::into_continuous` consumes the warmed engine. Its sessions and scratch
stay together; the existing page state becomes owner zero and one additional
page state is allocated at startup for owner one. Each owner retains its decoded
frame, detected boxes, script flags, token state, text assembly and position in a
sorted line index. The borrowed input and response buffer remain owned by the
original admission reservation.

One `advance` call performs a preparation stage, a stateless line-model call,
math for one page, or page finalization. Before the next operation the server
fills free slots from the existing FIFO without waiting for a peer. Ready owners
alternate as the first row's owner. A compatible peer can supply subsequent
rows when both complete reference groups have the same padded width. Groups
are never split or repadded to accommodate another owner. Each owner’s group retains the reference 1.3 width ratio using the 64-column
minimum proxy. A combined invocation requires equal padded widths, at most
64 total rows and at most 40,960 raw width columns; its combined width ratio
can exceed 1.3 because the original groups remain intact. The proved `BatchPlan`
account checks the actual aligned tensor scratch separately. A wide owner can
become the next anchor; narrow work cannot indefinitely prevent its selection.

The bound is between operations, not a fixed wall-clock service guarantee.
Preparation may run the detector's bounded second pass. Optional math still
executes one page's decoder loop before the other owner advances. This pilot is
stateless cross-request batching; it does not continuously refill math caches.
Cancellation is checked between operations. An in-flight native call retains
both reservations, even if one client disconnects. Each selected operation arms
the existing terminal watchdog with its own deadline, or the earliest deadline
among the participating rows. A waiting, unrelated peer does not shorten that
operation's watchdog deadline. An owner that expires between operations has its
output discarded while its live peer can continue. If the watchdog has already
made the service terminal, the current native call must return before either
reservation is released; the next safe boundary rejects retained work and starts
no further operation or FIFO refill. Shared runtime failures are explicitly
reported as inference failures. Native/internal faults keep the existing
fail-closed worker policy.

Draining stops new HTTP admissions. Previously admitted uploads can still enter
the FIFO and both active owners can finish. Completion uses the ordinary
response path: the reservation lasts until the final response byte reference
drops. Readiness changes before drain completion, and the extra page allocation
must complete before the experimental worker reports ready.

## Memory and telemetry accounting

The additional page's token/position matrices reserve 6 MiB; its two text arenas
reserve 8 MiB. Fixed box, metadata and index arrays add smaller bounded storage.
A second decoded grayscale frame can be retained, subject to the existing
25-million-pixel image bound. Decode and geometric transforms have temporary
native allocations too. These are source bounds, not a measured process RSS.
The comparison must retain actual peak cgroup memory, RSS and OOM counters.
Native workers keep their explicit 8 MiB stack reservation.

`continuous_invocation` is emitted only after an actual model call returns. It
contains bounded model labels, both owners' row counts, padded width and elapsed
time; no request identifiers, pixels or recognized text. Shared tracing spans
record two `follows_from` owner links. Exporting both requires the tracing
follow-up's bounded SDK link cap of two and its actual OTLP two-link regression;
the earlier SDK cap of one could discard a link. Optional tensor diagnostics additionally
record sequence/frame/line/slot maps, so raw rows can be compared by identity;
these opt-in diagnostic files contain document-derived tensors and do not belong
in production telemetry.

Per-owner valid and padded column counts cover that owner's rows. A CTC call's
batch histogram entry is assigned once to the rotating anchor, so summing
successful owners does not double-count a shared invocation. Calls attached to
a failed owner are absent from successful-response metrics; invocation events
retain the actual calls. Both participating owners accrue the elapsed shared
stage duration. That is participation time: summing those stage durations is
not total CPU time. The experiment uses cgroup CPU counters for total CPU work.

## Acceptance and measurement

The native unit gate compares independent calls with a real two-owner call for
`script`, `rec` and `rec_cjk`. Two distinct TIFF pages provide different pixel
rows with the same canonical padded width. The original 90% second crop changed
script probabilities by about 0.00334 and failed the unchanged numerical gate.
[The retained counterexample](evidence/continuous-padding-20261018/receipt.json)
and [protocol amendment](../runtime/validation/audit/continuous-amendment-0001.json)
explain why different canonical widths are now rejected. Valid normalized inputs
must have identical f32
bits, discrete token IDs and valid lengths must agree exactly, and finite model
probabilities/decoded positions use the frozen `atol=rtol=1e-5` criterion. Only
padding is excluded from valid-value comparisons. Forced CJK routing here tests
that graph's scatter behavior; it is not a CJK accuracy claim.

Other native gates compare three complete requests with the ordinary engine,
including both distinct TIFF pages, a third admission refilling a completed
slot, and cancellation or invalid input affecting only its owner. The real
server process gate holds two admitted uploads through both drain-command and
SIGTERM paths, requires exact complete responses, confirms clean exit, and
requires real shared script and CTC invocation events.

The separate Criterion target `continuous` covers 24 combinations of two
selection arms, four line counts and three batch caps. Both arms construct and
sort from the same unsorted widths inside each measurement. This measures Rust
scheduler control cost, not OCR speedup. The standalone System allocator gate
requires all six allocation/reallocation/deallocation count and byte deltas to
be zero in its prepared two-owner selection region. It excludes page decoding,
model execution and native allocators. The existing 46-case profiling protocol
is unchanged; these 24 cases are a separate experiment.

The deployment decision additionally needs paired runs at actual one-CPU and
two-CPU total limits, the same admitted capacity and native settings, short/long
mixed requests and concurrency one/two/three. Report successful latency
quantiles with their sample counts, errors, throughput, CPU time, padding and
peak memory; preserve every timeout and rejection. Compare the exact ordinary
and experimental executable identities. Native-only timings and the full
gateway/Tika/bridge path must remain separate. This source snapshot does not
claim either has been measured.

## Math compaction is a separate experiment

The pinned [Indra math decoder](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py)
uses one scalar step position and a self-attention cache time axis shared by
the batch. The native port supplies scalar `pos`, starts with
`[3, B, 8, 0, 24]` self caches and checks returned caches with time length
`step + 1`. A newly inserted BOS row at the current global step would have the
wrong position and an invalid prior history. Zero-filling that history is not
equivalent to an empty cache.

Removing completed rows is feasible without arbitrary-age refill: gather the
still-live rows of tokens, cross caches, mask and self caches, preserving the
original row map and shared step. Its tensor copying and possible lost vector
efficiency must be measured against saved decoder row-steps. The existing
heterogeneous fixture's 202 post-completion row-steps out of 360 quantify work
that could be avoided; they are not a latency reduction or a measured speedup.
Compaction implementation, exact row/cache parity and paired timings remain
separate gates. No math export or default decoder behavior changes in this
stateless pilot.

## Native experiment controller checkpoint

The source now includes `continuous_experiment`, a separate diagnostic example,
and `scripts/benchmark-continuous.sh`. They implement the native-only scheduler
layer of the frozen protocol. This is executable experiment infrastructure;
there are no accepted new measurements in this checkpoint.

The native example uses real `Engine`/`Continuous` sessions, the exact twelve
compiled-in fixture admissions, a bounded closed-loop concurrency of one, two or
three, at most two active owners, and an ordinary unmeasured reference pass.
It retains every planned request, output hash, equality/failure result, queue and
completion latency, per-request recognition batch counts and valid/padded columns.
The continuous invocation record includes actual returned model rows, shared
invocations and padded widths. Ordinary invocation diagnostics are unavailable;
its per-request recognition histograms remain available. Neither missing native
copy instrumentation nor unavailable invocation data is an observed zero.

Both arms use instrumented mimalloc. The existing `stats_alloc` dependency's
historically named `nightly` feature only exposes its const generic constructor;
that source compiles with the pinned stable Rust compiler. Counters cover Rust
global allocation during warmed scheduling and hashing, excluding JSON output,
model initialization and ONNX/OpenCV native allocations. They do not establish
zero allocation across the complete native stack. All arenas and the second
owner are allocated before timing. Per-request latency ends at completion before
hashing; complete-workload time includes hashing. Nearest-rank p50/p95/p99 retain
their finite successful counts and failure denominator; twelve requests do not
resolve stable production p99 behavior.

Build in the same native static-link environment as the other diagnostics:

```sh
source /native/cargo-env.sh
cargo build --locked --profile profiling --manifest-path runtime/Cargo.toml \
  --target x86_64-unknown-linux-gnu -p indra-engine --example continuous_experiment
```

Before execution, retain the producing build identity JSON with `schema_version:1`,
`source_sha`, `binary_sha256`, `protocol_sha256`, `native_materials_sha256`,
`native_equivalence_passed:true`, `native_equivalence_receipt` and
`native_equivalence_receipt_sha256`. The equivalence receipt must be a regular
sibling file preserving the actual passing numerical/ownership/lifecycle gate
evidence; its full bytes are retained with the experiment. A final-text match
alone cannot substitute for tensor/logit parity. Failed native gates make the
experiment ineligible; preserve those failures and fix the implementation before
producing an eligible new attempt. Do not relax frozen floating tolerances or
occupancy/performance thresholds.
Those SHA-256 values must identify the actual built binary, frozen protocol and
retained complete native material bundle. Do not manufacture a build receipt from
an unrelated source checkout.

```sh
scripts/benchmark-continuous.sh plan /absolute/new-plan-directory
scripts/benchmark-continuous.sh run /absolute/static/continuous_experiment \
  /absolute/build-identity.json sha256:<immutable-local-image-config-id> \
  /absolute/verified-model-snapshot /absolute/new-results <source-commit>
```

The controller verifies fixtures, static ELF and producer identities, records the
host and model-file inventory, alternates paired arm order, creates a new
network-disabled Docker container per arm/repetition, and sets aggregate 1/2 CPU,
affinity, 3 GiB, zero swap, 128 tasks and an 8 MiB stack. The binary verifies actual
CFS/memory limits and retains startup and measured-region cgroup counters and
process high-water status. The immutable local image ID identifies this diagnostic
container; it is not evidence of an independently signed release. A finite timeout
retains container diagnostics and stops the owned workload. Failed and interrupted
cells remain visible. The complete plan has 240 cells: 120 native and 120 Tika.

The controller intentionally exits nonzero with a partial-study index: the
actual all-process Tika HTTP cells, exact numerical row-identity gates, separate
same-age math compaction experiment and final paired engineering-screen analysis
remain required. Do not use native-only success, standalone state-machine tests
or a type-only build to close those gates or enable the production default.


## Whole-stack Tika experiment driver

The separate Rust gateway example `continuous_http` supplies the Tika layer. It
starts the real native server and the gateway's forked Tika process, using the
same bridge, Java, JAR and native executable paths as the trace acceptance image.
Every helper and HTTP load generator belongs to the same Docker cgroup. The
normal arm records ordinary reference responses; the experimental arm obtains
those same references first, restarts native with the explicit continuous setting,
then warms each selected fixture before measurement. Neither arm exports OTLP or
Sentry data during timing. Gateway admission is three, native admission is one
worker plus two queue entries, and both arms keep batch64/math=false/spinning=true.

The fixed three-client loop retains all twelve outcomes. Successful timings end
only after the entire HTTP response and transport complete; truncated or oversized
responses fail. Fixture MIME types are preserved, including PNG versus TIFF.
Client allocator counters cover the Rust HTTP driver, not allocations in its
child processes. Response-buffer copy bytes are recorded; unknown native copies
remain null. All process statuses and the common cgroup's counters/peak are kept.

Native completion columns and actual shared model-row histograms are recovered
from the native process's structured logs after shutdown flush. Filtering uses
**event creation timestamps**, not asynchronous file-write offsets, so delayed
warmup messages cannot enter timed counters. Dropped logging records or malformed
ownership counters make the diagnostic sample ineligible. Raw component logs
remain available. The nine source tests check HTTP framing/MIME/caps, compiled
fixture hashes/schedule, timestamp exclusion and ownership/drop invariants;
they are not measurements of native inference.

Build the HTTP driver with the same static GNU Rust settings as the gateway:

```sh
RUSTFLAGS='-Ctarget-feature=+crt-static' cargo build --locked --release \
  --manifest-path runtime/Cargo.toml -p indra-gateway --example continuous_http
```

Assemble or reuse the diagnostic all-role image from
`docker/trace-acceptance.Dockerfile`, selecting the corrected native image and
exact Tika/gateway tools identities. The controller mounts the newly built static
driver at `/experiment`, so no shell or compiler enters the runtime image. In the
HTTP driver's producer metadata, add `role_sha256` with exact `gateway`, `native`,
`java`, `tika` and `bridge` SHA-256 values. At runtime the driver hashes all five
executed files; the controller rejects any mismatch with those producing values.
An older native binary cannot qualify merely because the client source is newer.

```sh
scripts/benchmark-continuous.sh run-tika /absolute/static/continuous_http \
  /absolute/tika-build-identity.json sha256:<immutable-all-role-image-config-id> \
  /absolute/verified-model-snapshot /absolute/new-tika-results <source-commit>
```

Run native and Tika layers as separately named immutable attempts. Each records
all 240 planned cells and explicitly leaves the other layer unexecuted; the
selected layer's `assigned_layer_complete` records its own completion. An exit
status of1 keeps the **whole study** partial even if all selected-layer cells
passed. Admit both layer artifacts together, verify every paired repetition and
apply the unchanged frozen throughput/latency/CPU/memory screen before drawing
conclusions. Math row compaction still needs its own implementation and native
experiment. Production continuous batching remains opt-in after a synthetic
pilot, including after an engineering-screen pass.


## Local execution admission

The prospective [execution amendment](../runtime/validation/audit/continuous-execution-amendment-0002.json)
keeps the frozen schedule and decision thresholds. Before either diagnostic driver
loads models, the controller binds its live host PID to the exact labelled Docker
scope and checks every ancestor CPU and memory cap. The driver requires an exact
`--host-admission` marker within 30 seconds. This setup precedes warmup and timing.
All Docker control calls have finite deadlines. Cleanup checks the full container
ID, image and owner label, verifies stop/removal, and fails on uncertainty.

The local comparison admits distinct physical cores sharing one socket and L3,
then retains raw topology, all controller/lock hashes and the original native
equivalence receipt. It requires fresh producer receipts for both changed diagnostic
executables. These controls do not supply the missing paired measurements.
