# Experimental batching across two requests

The opt-in worker can put compatible lines from **two admitted requests in one
ONNX Runtime invocation**. It shares the script classifier and the ordinary or
Chinese CTC recognizer, then scatters results back by owner and original line.
A completed slot can accept the next FIFO request while its peer continues.
The ordinary worker remains the default. The frozen quality study and its eight
arms do not enable this experiment.

The source and acceptance gates are implemented. Native execution, paired
one/two-CPU timings and a decision about deployment remain pending. A successful
type check or a queue containing two requests does not establish a shared native
call. The [prospective protocol](../runtime/validation/continuous-protocol.json)
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
rows, alternating while both have compatible work. The batch retains the
reference text policy: at most 64 rows, a 1.3 width ratio using the 64-column
minimum proxy, and at most 40,960 raw width columns. The proved `BatchPlan`
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
rows. A prespecified 90% crop for the second diagnostic row forces different
padding in the combined batch. Valid normalized inputs must have identical f32
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
