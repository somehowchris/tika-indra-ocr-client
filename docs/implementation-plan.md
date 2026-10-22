# Implementation and evidence roadmap

[Issue #13](https://github.com/somehowchris/tika-indra-ocr-client/issues/13) remains the authoritative requirement index. Implemented source, accepted diagnostic evidence and release/scientific acceptance are separate milestones. The [results ledger](results.md) records their current scope and producing identities; [development.md](development.md#review-order) lists the consolidated review order. No PR is merged automatically.

## Workstreams

| Issue | Scope | Current evidence and remaining acceptance |
|---|---|---|
| [14](https://github.com/somehowchris/tika-indra-ocr-client/issues/14) | Architecture, modules, ownership and resource limits | Rust source and independent reviews retained; later changes preserve bounded ownership contracts |
| [15](https://github.com/somehowchris/tika-indra-ocr-client/issues/15) | OCI identity and ordered Tika shutdown | Descriptor-chain correction and real Compose acceptance pass. Historical Kubernetes OCR/drain/terminal/cleanup now passes both caps; current-source release acceptance remains required |
| [16](https://github.com/somehowchris/tika-indra-ocr-client/issues/16) | Model cache, configuration, CLI and generated API | Real cold/warm reuse, partial/hash rejection, release dotenv, typed configuration and real-handler gates pass; explicit cache permission/offline negatives and current release acceptance remain tracked |
| [17](https://github.com/somehowchris/tika-indra-ocr-client/issues/17) | Useful end-to-end telemetry | Gateway/Java/bridge/native graph and the latest both-cap Sentry correctness checks pass; paired telemetry overhead remains pending |
| [18](https://github.com/somehowchris/tika-indra-ocr-client/issues/18) | Native planner and strengthened proof | Uncached 16-file Creusot proof and real native parity/allocation gates pass |
| [19](https://github.com/somehowchris/tika-indra-ocr-client/issues/19) | Profiling and measured optimization choices | Frozen `2193324` profile: 46 Criterion cases per cap, four admitted/visually reviewed captures. All 128 ablation processes complete; no candidate passes the promotion policy |
| [20](https://github.com/somehowchris/tika-indra-ocr-client/issues/20) | Continuous batching and math compaction | Actual shared-owner numerical/refill/cancellation/lifecycle gates pass; All 240 native/Tika cells and 2,880 responses admitted; frozen performance screen fails. Same-age math experiment complete with regressions; both features stay opt-in |
| [21](https://github.com/somehowchris/tika-indra-ocr-client/issues/21) | Static signed six-image release and native build reuse | First `2193324` release failed at unsupported CRI flag after Compose passed. Historical replay passes; source-`60c849a` release failed before signing; the consolidated release and explicit SDK/Chef proof remain pending |
| [22](https://github.com/somehowchris/tika-indra-ocr-client/issues/22) | Frozen corpus provenance | Main/laboratory inputs admitted. [Main audit](../evaluation/audits/main-metadata-recovery-20261019/README.md): 113/119 originals verified (97 data + 16 exact metadata), six ancillary metadata exclusions; original 119-entry lock and scored inputs unchanged. Laboratory amendment separately resolves two unavailable metadata bodies |
| [23](https://github.com/somehowchris/tika-indra-ocr-client/issues/23) | Amended 24-cell comparison | Local Docker aggregate-cap controller implemented; execution waits for a fully accepted signed candidate index |
| [24](https://github.com/somehowchris/tika-indra-ocr-client/issues/24) | Quarto/Seaborn analysis and visual verification | Historical plots rendered/verified; new 24-cell results, 60 paired comparisons, final visuals and narrative remain pending |
| [25](https://github.com/somehowchris/tika-indra-ocr-client/issues/25) | Development/deployment/operation handoff | Current setup and scoped evidence documented; final candidate identities, measured results and complete handoff remain open |

## Review sequence

The [development guide](development.md#review-order) describes the consolidated runtime, delivery and evaluation reviews. The [history index](history.md) retains the full earlier PR sequence and immutable source. A draft review is complete only when its own acceptance gates pass; PR numbers and implementation presence do not establish measured results.

## Current release and scientific dependency

The descriptor-level OCI mismatch is corrected. Source `2193324bf42984c9dd3009fd4226b96f2379fad3` passed actual Compose accepted-work shutdown in [run 39166554904](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39166554904), then failed before Kubernetes workload admission because pinned `crictl version` rejects `-o json`. Python/Tesseract reference, full trace and signing steps in that failed run were skipped.

The [source-`5ae8b82` run 39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394) failed at CRI client parsing. [Historical replay](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/docs/evidence/kubernetes-historical-2026-10-19/README.md) now passes both-cap Kubernetes acceptance. [Source-`60c849a` validation 39232133426](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39232133426) passed native, Compose, Kubernetes and reference gates, then failed during trace-image assembly before signing. [Focused diagnostic 39252028096](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39252028096) subsequently passed the unchanged trace gate at both CPU caps using the exact source-41 runtime and separately identified controller/helper. This diagnostic establishes that assembly correction; it does not establish a signed consolidated release. Every production gate and signing step must pass for the new producer before exposing a trusted six-image index. The opt-in SDK/Cargo Chef proof then requires independent fresh-runner verification. A faster build is not established by moving cost into an uncounted producer or by reading a cache manifest.

The amended 24-cell main/laboratory matrix must consume the fully accepted signed candidates; an unsigned local diagnostic is not a substitute. Its admitted observations then feed the frozen paired-cluster report, complete failure coverage, Seaborn figures, Quarto rendering and final visual review. Continuous and telemetry synthetic studies remain distinct experiments, with their own exact drivers, resource admission and timing scopes.

## Review requirements retained

Actual shared-owner calls must preserve complete canonical reference groups and padding. Numerical comparisons bind owner/line/step identity, finite tolerances and discrete output decisions; compiler settings and request success alone cannot prove equivalence. Allocation counters cover their declared bounded operations, not total ONNX/HTTP allocations. Draining removes readiness and closes admission while accepted native work and response buffers retain ownership.

No evidence currently establishes a universally highest-accuracy setting, a generally superior two-CPU policy or an exhausted optimization space. Negative results and failed attempts stay in the record.
