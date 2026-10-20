# Documentation index

Two kinds of document live here. **Operate** pages tell you how to build, run, configure and verify the native Indra OCR stack with Tika 3.3.2. **Method and evidence** pages record how claims were tested, what was accepted and what remains open. The repository [README](../README.md) is the entry point; [results](results.md) is the acceptance ledger.

## Operate

| Page | Covers |
|---|---|
| [Run native Indra with Tika 3.3.2](container-deployment.md) | Docker Compose and Kubernetes deployment, resource profiles, probes, signed-image verification, OTLP and Sentry |
| [Rust OCR service](runtime-service.md) | `indra-server` operation: configuration precedence, deadlines, overload, probes, errors, telemetry |
| [Tika shutdown that preserves accepted parses](tika-gateway.md) | `indra-gateway` as the Tika entrypoint: readiness withdrawal, draining, admission, child supervision |
| [Immutable model cache](model-cache.md) | Fetch, verify and serve the pinned model snapshot offline |
| [Trace an actual Tika parse](distributed-tracing.md) | W3C context from the gateway through Java and the bridge to the native service |
| [Native text and Tika's XHTML boundary](tika-text-boundary.md) | Page separators and text normalisation between the native API and Tika |
| [Local Rust development](development.md) | Workspaces, local checks, the review order of the stacked pull requests |
| [Architecture and engineering contract](design.md) | Components, ownership rules and the code rules enforced by CI |
| [Reproducible native dependency build](native-build.md) | ONNX Runtime and OpenCV compilation, recorded inputs, resource scope |
| [Reproducible application compilation](build-reproducibility.md) | Application compiler stages and their clocks |
| [Bind images at the correct OCI level](oci-identity.md) | Index, manifest and configuration digests in deployment and release gates |
| [Static GNU bridge: source and relinking](relinking.md) | glibc source and relinking materials shipped with the bridge image |
| [Native GNU runtime: source, notices, relinking](runtime-relinking.md) | The same obligations for the native service and gateway images |

## Method and evidence

| Page | Covers |
|---|---|
| [Results and current acceptance ledger](results.md) | Accepted evidence, failed attempts and pending gates by exact run and source |
| [Implementation and evidence roadmap](implementation-plan.md) | Requirement index and the state of each work package |
| [Implementation history](history.md) | Archived packets and removed baseline-era records with their Git IDs |
| [Native image OCR engine](native-engine.md) | Pipeline port, parity harness, allocation gates and CPU dispatch |
| [Batch arithmetic verification](batch-proof.md) | The Creusot proof of the batch plan |
| [Native profiling and control-path measurements](native-profiling.md) | Flame graphs, hotspot tables and their admission |
| [Native one-factor ablations](native-ablations.md) | Prospective ablation protocol and outcomes |
| [Experimental batching across two requests](continuous-batching.md) | The opt-in two-owner pilot and why it stays disabled |
| [Same-age math decoder experiment](math-compaction-experiment.md) | Opt-in math-row compaction and its gates |
| [Whole-stack local telemetry experiment](telemetry-overhead-study.md) | Telemetry overhead protocol for the local Docker controller |
| [Admit a signed release to the frozen HTTP reference replay](current-http-reference.md) | Replaying the frozen HTTP reference against a signed candidate |
| [Independent completed-profile admission](profile-admission-checklist.md) | Admission checklist for completed profiling runs |
| [Private profiling bundle transport](profile-private-bundle.md) | Moving a host profiling bundle without widening its ownership |

Evidence packets referenced by these pages live under `docs/evidence/`, `runtime/validation/`, `evaluation/` and `validation/`; each packet carries its own `SHA256SUMS` or receipt.
