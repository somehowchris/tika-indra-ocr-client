# Architecture and engineering contract

The application replaces Tika's Tesseract executable contract while keeping **Tika 3.3.2** responsible for parsing, embedded-document traversal and PDF rasterization. The OCR service is a persistent native Rust process. It verifies an immutable local model snapshot, warms all configured workers, admits bounded requests and exposes readiness only when it can perform OCR. The separately packaged Python service is a comparison reference.

## Modules and boundaries

| Component | Responsibility | Ownership boundary |
|---|---|---|
| Root `indra-tesseract` package | Implement the Tesseract CLI subset Tika invokes; stream encoded image bytes and validated UTF-8 output | One bounded transfer; no dependency on the HTTP/native runtime workspace |
| `indra-gateway` | Public Axum endpoint, bounded Tika requests, Java process-group supervision and ordered drain | Own Java lifetime and retain admission until response bytes are released or the parser group is reaped |
| `indra-server` | Configuration, probes, API, bounded upload/queue/response ownership and native-worker supervision | Lease follows the input through upload, queue, synchronous inference and final response |
| `indra-engine` | Image decode, segmentation, reading order, script/CTC recognition and optional math decoding | Preallocated bounded storage and concrete native types; sessions stay on dedicated threads |
| `indra-core` | Admission/drain transitions, fixed-capacity FIFO and checked batch-work planner | Small deterministic operations with independent allocation/proof evidence |
| `indra-models` | Explicit acquisition and read-only verification of a pinned model revision | Verify size/hash before atomic publication; serving never downloads |
| `indra-telemetry` | Contextual tracing, real OTLP/Sentry transport, finite labels, redaction and bounded flush | Export queues cannot expand request admission or retain document payloads |
| `evaluation/runner` | Frozen input preparation, request journals, Unicode scoring, paired comparisons and performance extraction | Immutable file identities and explicit missing/failed attempts; no serving dependency |

```mermaid
flowchart TD
  Caller["Document caller"] --> Gateway["Rust Tika gateway"]
  Gateway --> Java["Private Tika 3.3.2 JVM"]
  Java --> Bridge["Static executable adapter"]
  Bridge --> OCR["Native Rust OCR service"]
  Cache["Verified model cache"] --> OCR
  Gateway --> Evidence["Bounded operational telemetry"]
  OCR --> Evidence
```

Java's private listener and the OCR listener are inside one network namespace. The gateway is the public admission point. The model initializer runs before serving and is not part of a warm inference measurement. There is no external broker, model registry service, GPU requirement or Python dependency on the production OCR path.

## Admission, readiness and shutdown

Readiness means the process has completed required startup and is accepting work. Temporary queue saturation is a request-level overload response; it does not oscillate readiness. Draining removes readiness and rejects new admission before waiting for accepted work. Liveness remains separate. A client's cancellation cannot free storage still in use by a native call or Java parser.

The default OCR service has one worker and three total admission leases: the active native request plus two queued/uploading responses. Limits are fixed and validated. The gateway independently limits active parsing requests and connections, and streams response chunks through a bounded channel. These are memory and ownership contracts, not promises that arbitrary input finishes within an unbounded deadline.

Native computation runs on dedicated OS threads, never inside a Tokio future or a Rayon pool reached from async work. Each worker explicitly reserves an 8 MiB stack: the retained real container failure showed an overflow on the default worker stack. The service rejects obvious worker/thread oversubscription. CPU flags do not enforce quotas; Docker/Kubernetes/cgroup validation does that separately.

The gateway owns the JVM process group and reaps children. If a response ends ambiguously while native/Java work might still run, it keeps the reservation until the process ownership boundary is resolved. A synchronous native call cannot safely be force-unwound; a finite watchdog changes status and terminates the failed process after bounded cleanup.

Compose keeps OCR as the network-namespace owner and stops its dependent Tika container first. Kubernetes uses an ordinary cache init container, then a restartable OCR init sidecar, then the main Tika container. Its native-sidecar termination order keeps OCR available while Tika drains. Do not install an OCR preStop hook that removes this dependency early. The shipped 450-second Pod grace covers both bounded stages. See the [Kubernetes sidecar lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/#sidecar-containers-and-pod-lifecycle) and [actual deployment](../deploy/base/deployment.yaml).

## Boundary limits and final owners

These are code defaults; the shipped Compose/Kubernetes profiles explicitly
select their aggregate resources and math setting. Configuration validates
changes before serving. The full schemas remain in the linked operation guides.

| Boundary | Framing, bounds and deadline | Final owner and failure behavior |
|---|---|---|
| Public caller → gateway | HTTP/1 with unambiguous fixed-length uploads; 2 active parses, 16 connections, 64 MiB input and 16 MiB streamed output by default; 5 s first-header, 15 s upload and 300 s absolute request deadlines | Input, backend and frontend owners all retain the slot. Incomplete uploads never start Java. Client disconnects after backend start drain to a bounded sink; ambiguous upstream EOF/deadline is terminal and retains capacity until Java's group is reaped. [Contract/source](tika-gateway.md#http-and-ownership-contract) |
| Tika → executable adapter → native HTTP | Loopback literal address; one image with exact positive Content-Length, at most 20 MiB; 70 s adapter exchange by default; validated UTF-8 output, at most 4 MiB | The dependency-free adapter streams bounded chunks. Native admission owns uploaded bytes through FIFO, synchronous work and the final response reference. Invalid framing/media/encoded images have finite typed errors; saturation/drain returns 503, upload/request deadline 504. [Protocol](runtime-service.md#api-contract), [adapter](../src/main.rs) |
| Native worker → ONNX/OpenCV | One dedicated worker by default; 8 MiB worker stack, at most 64 rows and checked tensor-work bounds; at most 16 TIFF pages, 25 million decoded pixels and 1,024 lines per page | Sessions and arenas stay on their synchronous owner. Deadlines are checked between native calls; a hung call cannot release its lease or be force-unwound safely. A terminal watchdog removes readiness and ends the process after bounded cleanup. [Engine limits](native-engine.md#batching-and-memory), [shutdown](runtime-service.md#probes-and-graceful-shutdown) |
| Initializer → model cache | Explicit single-writer acquisition; immutable revision, exact byte counts/SHA-256, 10 s connect and 300 s request timeout by default | Verified temporary files publish atomically; failed replacement preserves the valid old file. Serving is read-only and performs no acquisition. No distributed cache lock or live model replacement is implied. [Cache](runtime-service.md#model-cache-and-startup), [source](../runtime/crates/indra-models/src/lib.rs) |
| Process → exporters | Finite trace/metric exports; 512-entry trace and log queues, 64-span batches and 16 queued Sentry events by default; 2 s configured export timeout | Dedicated transport/log threads own bounded metadata only. Documents, text and credentials never enter payloads. Dropped logs are counted; shutdown flush is bounded and included in deployment grace. [Wire/privacy contract](runtime-service.md#tracing-metrics-and-sentry) |

The default gateway upload pool reserves 128 MiB and native input/output pools
reserve 72 MiB, before model sessions, native tensors, decoded frames, thread
stacks, JVMs, temporary files, transport buffers or exporters. These are capacity
bounds, not RSS measurements. The gateway JVM starts with a 128 MiB heap ceiling;
its forked parser has a separate 512 MiB ceiling. Native temporary allocations and
Java/native non-heap memory still count against the same deployment budget.
[Compose](../compose.yaml) and [Kubernetes](../deploy/base/deployment.yaml) partition
3 GiB across Tika/native containers; scientific execution instead measures all
application processes and its load generator in one 1/2-CPU, 3-GiB parent cgroup.
Neither heap settings nor thread counts enforce that aggregate limit.

## Batching and continuous work

The existing engine dynamically groups width-sorted lines within a document, subject to batch count and tensor-work limits. It pads a batch to the selected maximum width and reuses bounded arenas. The native grouping policy and the aligned memory safety planner are distinct: changing the latter must not silently change membership, order, padding policy or model inputs.

The disabled-by-default [two-owner pilot](continuous-batching.md) combines compatible script/CTC rows from two admitted requests in a real native invocation, preserves each complete canonical group and routes outputs by owner/line. Free slots refill from the existing FIFO between operations. Cancellation retains buffers until native return; draining closes new admission while accepted uploads and both owners finish. Its [240-cell synthetic comparison](../evaluation/continuous/evidence/2026-10-19-pilot/README.md) admits all responses but fails its predefined performance screen. Complete-Tika throughput is lower in every tested stratum, so the ordinary worker remains the default. No timer, extra queue or arbitrary-age math refill is introduced.

The pinned math decoder takes one scalar position for the batch and uses self-cache tensors shaped `[3, batch, 8, steps, 24]`. Fresh rows of a different decoding age cannot simply reuse completed slots. [Same-age completed-row compaction](math-compaction-experiment.md) is a separate measured experiment with exact row/cache parity and observed two-CPU regressions. Arbitrary-age refill requires a changed graph/cache contract and new numerical/quality evidence. Existing heterogeneous six-row evidence contains 202 post-completion row steps out of 360. That is redundant row work under this policy, **not a predicted 56.1% latency saving**. See the [math implementation](../runtime/crates/indra-engine/src/math.rs) and [recorded termination evidence](../runtime/crates/indra-engine/tests/evidence/2026-10-18-parity/math-termination.json).

## Rust ownership and allocation policy

Use concrete types and enums, borrowed slices, bounded arrays, checked arithmetic and structured `thiserror` errors. Use `const fn` where it expresses a real compile-time invariant and `static` for shared global values. Avoid application `dyn`, growing `Vec` queues, `clone`/`to_owned` payload duplication, unnecessary `Box`, implicit large-value copies and blanket `anyhow` errors. The workspace lints reject implicit/redundant clones and large pass-by-value types.

These rules must be evaluated at an explicit boundary. OpenCV and ONNX Runtime own internal buffers; decoded image transformations and model outputs have real storage costs. Borrowing an image view, moving a tensor handle and copying pixel data are different operations. Do not assert that the entire process allocates nothing or eliminate an ownership check just to satisfy a lexical search. Track justified native/library allocations separately and measure churn in the operation being changed. The service uses mimalloc; the dependency-free bridge retains its system allocator and separately demonstrates zero heap churn in its streaming control operation.

## ONNX, SIMD and optimization

The build pins ONNX Runtime 1.31.0 and OpenCV 5.0.0 source revisions, tools, downloaded archives and package snapshot. OpenCV uses a portable SSE3 baseline and runtime dispatch through AVX512_SKX; ORT uses its own MLAS dispatch. Disabling a generic build AVX option does not imply every specialized MLAS kernel is absent. IPP is a pinned upstream binary component and must be disclosed as such. No release image is built with host-specific `target-cpu=native`.

One/two CPU profiles use explicit native thread limits and sequential graph execution. ORT spinning can reduce wakeup latency while consuming quota, so it is an ablation rather than an assumed universal improvement. [ORT's thread documentation](https://onnxruntime.ai/docs/performance/tune-performance/threading.html) describes that tradeoff. The release Rust profile uses thin LTO, one codegen unit, checked arithmetic and aborting panics; fat LTO is a measured candidate. Profiling builds retain symbols. Source/build flags alone establish no measured throughput claim.

## Configuration, API and observability

`config` and Clap merge defaults, an explicitly named TOML file, environment and flags, in that order. Typed validation rejects unknown fields and incompatible limits before startup. Miette diagnostics show safe paths and types without echoing credentials. Dotenv is debug-only. Cache fetch/verify, configuration checking, OpenAPI generation, shell completion and probe/drain commands work without a runtime shell.

Tracing covers admission, queue time, model startup, native stages, worker identity, batch width/padding and finite error classes. OTLP exports real protobuf traces and metrics; Sentry emits contextual incidents for terminal failures. Wire tests inspect these payloads and their redaction. External exporters are bounded and do not see images, OCR text, filenames, authentication headers or arbitrary baggage. The [Tika source overlay](distributed-tracing.md) carries validated W3C context through request-scoped Java parse/process spans and the bridge into native OCR, including concurrent requests. It never mutates process-global context. Real collector gates verify the graph, privacy and shutdown flushing; paired overhead remains a separate measurement. Sentry-only gateway failures retain their finite HTTP request ID without requiring an OTLP exporter.

## Static packaging and evidence

The Rust runtime stages are scratch; Java and the Python reference use digest-pinned Debian distroless images. No Alpine stage is permitted. A build gate inspects every shipped executable for absent `PT_INTERP` and `DT_NEEDED`, executes the actual artifact, and retains exact source/relink materials and third-party notices. Static GNU deployment still requires explicit trust stores and licensing materials.

Release verification must distinguish an OCI index, selected platform manifest, config and the identity Docker records on the created container. Compare the verified descriptor chain rather than assuming every Docker `.Id` or `.Image` is the same kind of digest. Candidate tags are discovery aids; immutable digests, bounded signature verification, decoded attestation predicates and retained gate records establish what was executed and signed.

## Verification boundaries

[Creusot](https://creusot.rs/) proves the batch planner's specified arithmetic/state contract. It does not prove native libraries, all scheduler behavior, lifecycle or OCR accuracy. Native stage parity proves agreement with the pinned reference on its declared fixtures. Process tests prove their exercised lifecycle transitions. Scientific quality and performance require the separately frozen corpus and aggregate cgroup measurement. Each result records its own source revision, inputs, scope and limitations in [the results ledger](results.md).
