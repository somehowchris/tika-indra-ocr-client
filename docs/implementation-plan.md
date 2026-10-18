# Implementation and evidence roadmap

The authoritative task index is [GitHub issue #13](https://github.com/somehowchris/tika-indra-ocr-client/issues/13). It maps every requested requirement to an issue, records dependencies and separates implemented source from accepted evidence. The original implementation request remains active; issues do not stand in for unexecuted work.

## Workstreams

| Issue | Scope | Dependency or evidence |
|---|---|---|
| [14](https://github.com/somehowchris/tika-indra-ocr-client/issues/14) | Architecture, module/ownership/limit contracts | Existing source and independent review |
| [15](https://github.com/somehowchris/tika-indra-ocr-client/issues/15) | OCI identity and real ordered Tika shutdown | Current release blocker; exact Docker/CRI identities and accepted-work lifecycle |
| [16](https://github.com/somehowchris/tika-indra-ocr-client/issues/16) | Model cache, configuration, CLI, generated API | Cold/warm/failure, release dotenv and real handler acceptance |
| [17](https://github.com/somehowchris/tika-indra-ocr-client/issues/17) | End-to-end context and useful measured telemetry | Request-scoped Java/CLI propagation; wire/redaction/overhead evidence |
| [18](https://github.com/somehowchris/tika-indra-ocr-client/issues/18) | Native planner integration and strengthened proof | Uncached Creusot plus rebuilt native parity/allocation checks |
| [19](https://github.com/somehowchris/tika-indra-ocr-client/issues/19) | Profiling and optimization decisions | 46 Criterion cases, four valid perf captures, paired ablations and visual review |
| [20](https://github.com/somehowchris/tika-indra-ocr-client/issues/20) | Bounded continuous batching and math compaction | Prove a real shared-owner model invocation, ownership, parity and mixed-load results |
| [21](https://github.com/somehowchris/tika-indra-ocr-client/issues/21) | Static signed six-image native release | Exact OCI chain, real acceptance, notices/relink inputs, OIDC/SBOM/evidence |
| [22](https://github.com/somehowchris/tika-indra-ocr-client/issues/22) | Corpus provenance and qualified coverage | Frozen source/reference identities and explicit HTML recovery decision |
| [23](https://github.com/somehowchris/tika-indra-ocr-client/issues/23) | Frozen 32-cell comparison | Signed candidates, main/laboratory admission, Rust journals/scores and cgroups |
| [24](https://github.com/somehowchris/tika-indra-ocr-client/issues/24) | Quarto/Seaborn analysis and visual verification | Admitted data, failure denominators, paired cluster inference and inspected HTML |
| [25](https://github.com/somehowchris/tika-indra-ocr-client/issues/25) | Final development/deployment/operation handoff | Tested commands, exact release identities, current results and clean PR stack |

## Uploaded recovery parts

The original stack remains in PRs 1, 3–10; [development.md](development.md#review-order) explains its order. The additional uploaded work is:

| Part | Review location | Status |
|---|---|---|
| Native planner, proof strengthening, profile export/FIFO/measurement source | [PR 11](https://github.com/somehowchris/tika-indra-ocr-client/pull/11), source `9d0f2ec2` | Draft; local checks pass, new hosted native/solver/profile gates pending |
| Architecture, native setup, evidence ledger, Quarto source and historical figures | [PR 12](https://github.com/somehowchris/tika-indra-ocr-client/pull/12), initial source `2ca56887` | Draft; eight source rows and 16 marks verified, standalone PNGs inspected; full new report pending |
| Bounded signing helper | [release recovery branch](https://github.com/somehowchris/tika-indra-ocr-client/tree/wip/recover-signed-release-20261018), first checkpoint `1fa36825` | Partial checkpoint; remaining release workflow/verifier reconstruction is tracked in #21 |
| Acquisition/schedule/index helpers | [evaluation recovery branch](https://github.com/somehowchris/tika-indra-ocr-client/tree/wip/recover-evaluation-execution-20261018), first checkpoint `d0bb6be7` | Partial checkpoint; full driver/aggregation/workflow and scorer followup are tracked in #23 |

The release and evaluation branches receive small follow-up commits as recovery
completes. Their existence does not imply completed native publication or a new
scientific run. No PR is merged automatically.

## First remaining blocker

The native service passes its real 1/2-CPU lifecycle gate. The full Tika run reaches
readiness but rejects a Docker index/platform identity mismatch before its drain
harness. Issue #15 records exact reproducing digests and requires a verified
descriptor chain, rather than relaxing identity checks. After that correction,
the actual Compose/Kubernetes accepted-work drain must be rerun before release
acceptance. [results.md](results.md) links the producing runs and retained files.

## Review corrections already incorporated

Independent review of issues #14/#18/#19/#20 against `9d0f2ec2` added two batching
requirements: a real model invocation must contain work from both admitted
owners, and changed batch shapes must use an owner/line/step comparison with
explicit padding/masks rather than an unchanged whole-array comparator. It also
distinguished zero-allocation control gates from model-verification allocation
observations and clarified that draining changes readiness/admission together.
