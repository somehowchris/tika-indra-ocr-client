# Implementation and evidence roadmap

[Issue #13](https://github.com/somehowchris/tika-indra-ocr-client/issues/13) remains the authoritative requirement index. Implemented source, accepted diagnostic evidence and release/scientific acceptance are separate milestones. The [results ledger](results.md) records their current scope and producing identities; [development.md](development.md#review-order) lists the actual stacked PR order. No PR is merged automatically.

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
| [21](https://github.com/somehowchris/tika-indra-ocr-client/issues/21) | Static signed six-image release and native build reuse | First `2193324` release failed at unsupported CRI flag after Compose passed. Historical replay passes; current source-`60c849a` release and explicit SDK/Chef proof remain in progress |
| [22](https://github.com/somehowchris/tika-indra-ocr-client/issues/22) | Frozen corpus provenance | Main/laboratory inputs admitted. Explicit pre-inference amendment resolved two unavailable HTML metadata bodies without changing scored inputs/references |
| [23](https://github.com/somehowchris/tika-indra-ocr-client/issues/23) | Frozen 32-cell comparison | Local Docker aggregate-cap controller implemented; execution waits for a fully accepted signed candidate index |
| [24](https://github.com/somehowchris/tika-indra-ocr-client/issues/24) | Quarto/Seaborn analysis and visual verification | Historical plots rendered/verified; new 32-cell results, 112 paired comparisons, final visuals and narrative remain pending |
| [25](https://github.com/somehowchris/tika-indra-ocr-client/issues/25) | Development/deployment/operation handoff | Current setup and scoped evidence documented; final candidate identities, measured results and complete handoff remain open |

## Uploaded implementation and continuation

The original architecture/model/static/native/service/deployment/corpus work remains in PRs 1, 3–12. Recovery PRs [26](https://github.com/somehowchris/tika-indra-ocr-client/pull/26), [27](https://github.com/somehowchris/tika-indra-ocr-client/pull/27) and [28](https://github.com/somehowchris/tika-indra-ocr-client/pull/28) preserve release, continuous-native acceptance and scientific execution source. The completion stack starts at [29](https://github.com/somehowchris/tika-indra-ocr-client/pull/29), which integrates those interrupted branches.

| Current review part | Scope | Accepted evidence / remaining boundary |
|---|---|---|
| [29–38](https://github.com/somehowchris/tika-indra-ocr-client/pull/38) | Convergence, full tracing, nextest, profile control, local Docker study, canonical two-owner planner, report/release admission, setup and exact correctness identities | Native output/stage replay, diagnostic tracing, current-server HTTP replay and real service/Compose evidence retained; candidate-specific Kubernetes/signing and full study remain separate |
| [39–41](https://github.com/somehowchris/tika-indra-ocr-client/pull/41) | Frozen build-cost protocol, exact Cargo artifact selection, material leaf and immutable native SDK | Historical missing-blob rebuild and actual local SDK admission proven; fresh seven-role/compiler/material equivalence and speedup pending |
| [42–45](https://github.com/somehowchris/tika-indra-ocr-client/pull/45) | Prespecified native ablations, live continuous resource admission, isolated math compaction, complete shutdown grace | Native numerical/lifecycle tests and isolated math gates pass; release defaults unchanged |
| [46](https://github.com/somehowchris/tika-indra-ocr-client/pull/46) | Complete capped native profile and private bundle transport | All 46 cases per cap/four captures admitted; original figures viewed, raw sample replay passes; unknown frames and host activity disclosed |
| [47](https://github.com/somehowchris/tika-indra-ocr-client/pull/47) | Purpose-owned cache policy and exact build-cost/Chef evidence | Original 4,436.7-second native rebuild proven; cache cause, reliable cross-runner persistence and candidate speedup remain unproven. Exporters default to disabled |
| [48](https://github.com/somehowchris/tika-indra-ocr-client/pull/48) | Bounded telemetry study and exact current-server HTTP reference replay | 28 selected native HTTP outputs pass; production server ELF and older diagnostic role producers stay explicit. Paired overhead requires later admitted execution |
| [49](https://github.com/somehowchris/tika-indra-ocr-client/pull/49) | Paired math evidence and strict ablation resource admission | Forty timed math arms plus six references/gates retained; two-CPU regressions prevent promotion. Completed ablation continuation preserves all 128 new processes and the earlier parser failure |
| [50](https://github.com/somehowchris/tika-indra-ocr-client/pull/50) | Supported CRI receipts and trusted opt-in signed SDK/Cargo Chef proof | Source/contracts uploaded; real cook, fresh hosted restoration, seven executable hashes, negative invalidation and paired net cost remain explicit gates |

The next twenty ordered drafts are [#51–#70](development.md#continuation-parts). They retain completed ablations, continuous/telemetry/Sentry evidence, the amended seven-binary content proof, opt-in native-input controls, build-test dependencies and the reviewed Kubernetes/CI corrections. Current acceptance remains explicit; publishing a draft does not complete its measured gate.

PR numbers identify uploaded review locations, not proof that every later local continuation or release experiment has already passed there. Complete producer hashes and raw receipts take precedence over branch names. The review sequence and exact bases are listed in the development guide.

## Current release and scientific dependency

The descriptor-level OCI mismatch is corrected. Source `2193324bf42984c9dd3009fd4226b96f2379fad3` passed actual Compose accepted-work shutdown in [run 39166554904](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39166554904), then failed before Kubernetes workload admission because pinned `crictl version` rejects `-o json`. Python/Tesseract reference, full trace and signing steps in that failed run were skipped.

The [source-`5ae8b82` run 39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394) failed at CRI client parsing. [Historical replay](evidence/kubernetes-historical-2026-10-19/README.md) now passes both-cap Kubernetes acceptance. Current [source-`60c849a` validation 39232133426](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39232133426) passed hosted correctness and is building native dependencies; every original runtime/signing gate must pass before it exposes a trusted six-image index. Its explicitly enabled SDK/Cargo Chef proof runs only after the unchanged production gates/signing, then requires independent fresh-runner verification. A faster build is not established by moving cost into an uncounted producer or by reading a cache manifest.

The 32-cell main/laboratory matrix must consume the fully accepted signed candidates; an unsigned local diagnostic is not a substitute. Its admitted observations then feed the frozen paired-cluster report, complete failure coverage, Seaborn figures, Quarto rendering and final visual review. Continuous and telemetry synthetic studies remain distinct experiments, with their own exact drivers, resource admission and timing scopes.

## Review requirements retained

Actual shared-owner calls must preserve complete canonical reference groups and padding. Numerical comparisons bind owner/line/step identity, finite tolerances and discrete output decisions; compiler settings and request success alone cannot prove equivalence. Allocation counters cover their declared bounded operations, not total ONNX/HTTP allocations. Draining removes readiness and closes admission while accepted native work and response buffers retain ownership.

No evidence currently establishes a universally highest-accuracy setting, a generally superior two-CPU policy or an exhausted optimization space. Negative results and failed attempts stay in the record.
