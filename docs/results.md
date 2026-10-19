# Results and current acceptance ledger

Updated 19 October 2026. This ledger separates accepted evidence from implemented but unexecuted work. The [historical baseline document](legacy-results.md) is preserved for its producing revisions; it is not the current native acceptance status.

## Native runtime and lifecycle

| Gate | Producing revision / run | Result and scope |
|---|---|---|
| Static native dependency build | [39086092581](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39086092581) | Passed pinned GNU native compilation and recorded archives/build inputs |
| Native Indra output/stage parity | [39092252965](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39092252965), source `9135086` | Passed 14 fresh cases and 850 intermediate tensors; see retained [parity records](../runtime/crates/indra-engine/tests/evidence/2026-10-18-parity/README.md) |
| Strengthened Creusot batch proof | [39146023143](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39146023143), tested merge `3d8a39fe`, PR head `9d0f2ec2` | Passed all 16 generated proof files, exact inverse projection and five ordinary tests; [full artifact and receipt](evidence/batch-proof-20261018/receipt.json) |
| Native service at 1/2 CPUs | [39143910423](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143910423), tested merge `cac99b31`, PR head `db645be7` | Passed 12 unit tests and one real native process lifecycle test per CPU setting in scratch containers |
| Rust runtime tests | [39143910642](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143910642), PR head `db645be7` | Passed after isolating allocation counters from libtest's concurrent reporting |
| Historical native gateway OCR acceptance | [39143910513](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143910513) | Passed; does not substitute for the deployment lifecycle gate |
| Fresh native diagnostic replay | Adapter `1d77970`, diagnostic ELF `26214af1…` | [14/14 cases and 850 tensors](../evaluation/parity-replay/evidence/2026-10-18/summary.json) pass unchanged tolerances; fresh reference text/tensor-manifest hashes match historical receipts. Newly generated arrays and changed reference interpreter remain explicit; this is not the HTTP-server ELF |
| Exact HTTP output of current native server | Client `697fec4`, server source `2193324`, diagnostic image `cbc65a…` | [28/28 frozen upstream responses](../evaluation/parity-replay/evidence/http-server-2026-10-18/acceptance.json) match at both CPU caps, including raw TIFF form-feed separators; distinct from the earlier 850-tensor diagnostic ELF |
| Full-stack tracing | Diagnostic source `060b4fe` | [Both CPU gates pass](../validation/tracing/2026-10-18/README.md): actual gateway → forked Java → bridge → native ancestry, concurrent parses, redaction and shutdown flushing; mixed/recovered build inputs are explicit |
| Full native Compose deployment | [39166554904](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39166554904), source `2193324` | Passed the actual accepted-work Compose shutdown step; separate [local mixed-build diagnostic](evidence/compose-canonical-diagnostic-20261018/README.md) also passes both caps |
| Kubernetes release acceptance | Same `2193324` run | **Failed before workload admission:** pinned `crictl version` has no `-o json` flag; [retained failure and correction](evidence/kubernetes-cri-version-2026-10-18.md). Later reference, trace and signing steps were skipped |
| Frozen native profile | Producing source `2193324` | [46 Criterion cases per CPU cap and four captures](../tests/evidence/2026-10-18-native-profile-baseline/README.md) admitted; zero lost samples; original figures visually reviewed. Synthetic diagnostics, not whole-Tika performance |
| Same-age math compaction | Producing source `dfef64b` | [40 timed arms plus six reference/gate roles](../runtime/validation/math-compaction/2026-10-18/paired-study/README.md) pass. Median cohort ratios compact/ordinary: 0.7805 at 1 CPU, 0.9294 at 2 CPUs; 3/10 two-CPU compact arms regress. Opt-in only |
| LTO/spinning/batch-cap ablations | Producing source `ad048ce` | [128 fresh processes / 4,096 recognitions](../runtime/validation/native-ablations/2026-10-18/README.md) and four native gate roles admitted. All four candidates fail the frozen conservative promotion policy; no default changed |
| Telemetry on/off experiment | Driver `d4d80fe`, diagnostic native ELF `c9cfe911…` | Both-cap local correctness passes with the real classified Sentry fault, trace/counter/privacy/resource/cleanup gates. Raw records remain under `telemetry-http-correctness-d4d80fe-http28-20261019` in the shared artifact directory; this is diagnostic evidence, separate from release acceptance. [Paired overhead protocol](telemetry-overhead-study.md) is frozen; overhead timings/analysis remain pending |
| Continuous native/Tika pilot | Driver source `85fef71` | Numerical canonical-row, refill/cancellation and server drain/SIGTERM gates pass at both caps. The 240-cell paired study has started; admitted aggregate performance results remain pending. Production pilot stays disabled |
| Signed six-image native release | [Corrected run 39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394), source `5ae8b82` | Validation run in progress at this update. All deployment/reference/trace/signing gates remain mandatory; no completed signed native candidate index accepted |
| SDK/Cargo Chef build reuse | [PR 50](https://github.com/somehowchris/tika-indra-ocr-client/pull/50), opt-in proof in the corrected run | [Real local SDK closure admission](../evaluation/build-cost/SDK.md) and [Chef metadata layout](../evaluation/build-cost/CHEF.md) pass. [Compiled reuse/fresh-runner seven-role equivalence and paired cost proof](../evaluation/build-cost/CHEF-COOK.md) remain pending; no candidate speedup claimed |
| Frozen multilingual main/laboratory study | Frozen acquisition source `667ec10`; [PR 33](https://github.com/somehowchris/tika-indra-ocr-client/pull/33) / [PR 35](https://github.com/somehowchris/tika-indra-ocr-client/pull/35) | [Inputs and acquisition amendment admitted](../evaluation/audits/acquisition-20261018/receipt.json); all 32 signed-candidate cells and final 112 paired comparisons remain pending |

The earlier service recovery receipt retains exact artifact identities, actual pass logs and the historical deployment mismatch evidence under [recovery-20261018](evidence/recovery-20261018/receipt.json). The earlier failure and controlled diagnostic remain under [native-service-stack-20261018](evidence/native-service-stack-20261018/receipt.json).

### Corrected worker stack failure

Real service acceptance originally aborted while initializing a native worker: its default stack was too small. The worker now explicitly reserves 8 MiB once at startup. The regression child sets `RUST_MIN_STACK=2097152`, so an ambient large-stack environment cannot conceal a missing explicit reservation. The rebuilt service passed native ready/drain/SIGTERM/output checks at both CPU caps. This proves the exercised lifecycle case; it does not establish that 8 MiB is the minimum possible stack or a throughput optimization.

### Historical identity failure and the current release gate

The older [39143910460 deployment run](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39143910460) rejected a healthy stack before lifecycle testing because it compared an OCI index identity with a selected platform manifest. That failure remains in the recovery receipts. The correction verifies the complete index → selected platform → config chain and binds the actual container, without accepting arbitrary tag membership. Later actual Compose acceptance passed; the historical identity mismatch is no longer the first remaining blocker.

The source-`2193324` release then reached Kubernetes and failed in its harness at `crictl version -o json`. The pinned client supports plaintext `version` and a separate client `--version`; [the correction](evidence/kubernetes-cri-version-2026-10-18.md) retains and validates both. Run `39213042394` uses that fix and must still complete Kubernetes, Python/Tesseract reference, full trace and signing gates. The failed release does not supply a signed accepted candidate, and its skipped gates are not inferred from local diagnostic passes.

## What native parity establishes

The retained study compared 850 tensors comprising 163,005,555 elements: 546 tensor checks used exact comparison and 304 used declared numerical tolerances. Values matched exactly in 805 tensors; that wording is not a bitwise identity claim for every floating-point payload. The largest absolute discrepancy was about `1.72e-5` in German reading-order scores and remained within the combined absolute/relative tolerance. All 14 expected text outputs matched the pinned reference.

The math gates include real batch sizes 1, 8 and 64 and heterogeneous termination. The six-row termination fixture uses 360 row steps, of which 158 precede or include row completion. The remaining 202 row steps motivated the now-completed same-age compaction experiment. Its measured ratios above include copies and preparation; the two-CPU regressions demonstrate why fewer row steps alone do not predict wall-time savings. It remains separate from production fresh-row refill and page/Tika throughput.

## Historical signed-container baseline

The signed release [39080351532](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39080351532), source `d6b37bc7623696ab2ee5d62c1f6af6d851438c6c`, compared Tesseract with **Python Indra**, using 15 mostly English synthetic cases and three repetitions. The [retained source summaries and CSV](../evaluation/results/signed-baseline-20261018/provenance.json) are bound to artifact `12119406416`, SHA-256 `a5fc83c8dd98d85db9363246394ea8a11343a4696171ebbbee701a3c75f04065`.

| Quota policy | CPUs | Tesseract responses/s | Python Indra responses/s | Tesseract p95 ms | Python Indra p95 ms |
|---|---:|---:|---:|---:|---:|
| Shared | 1 | 1.029 | 3.379 | 1701 | 824 |
| Shared | 2 | 1.330 | 4.899 | 2512 | 1077 |
| Deployment partitions | 1 | 1.028 | 1.457 | 1715 | 2557 |
| Deployment partitions | 2 | 1.324 | 2.693 | 2526 | 2835 |

Each cell had 45 successes and no HTTP failures. Concurrency was one at 1 CPU and two at 2 CPUs; the old load generator was outside the application's 3-GiB cgroup. The new study includes its load generator inside the common cap. These studies must not be pooled or presented as a native Rust comparison.

![Historical throughput](../evaluation/figures/signed-baseline-throughput.png)

![Historical p95 latency](../evaluation/figures/signed-baseline-p95.png)

The Seaborn source verifies each bar's source row, category, color, position, value and zero baseline. Both standalone charts have been visually inspected. The current [Quarto source](../evaluation/report/report.qmd) renders the historical data and explains the planned analysis; a complete new-matrix notebook and its rendered HTML visual review remain outstanding.

## Frozen quality study and interpretation

The main corpus has 33 primary pages in 19 document clusters and eight separate challenges. Its primary appearance mix is 19 printed, 11 handwritten and three mixed pages. English accounts for two primary pages, one cluster and 51 of 87,515 NFC reference letters: below 10% by each denominator. Public legal, historical, handwriting and script-specific inputs have documented reference qualifications; unknown training overlap and implementation exposure are disclosed.

The two-case Portuguese laboratory supplement remains separate and descriptive. It contains methods crops from two papers, not complete laboratory reports or clinical documents. Its protocol uses zero bootstrap resamples. The [pre-inference acquisition amendment](../evaluation/laboratory-supplement/audit/metadata-recovery-0002.json) already resolved its operational metadata blocker: two exact HTML originals were recovered, and two unavailable metadata bodies were explicitly excluded. Current replacement HTML is audit evidence only. Original audit records and all frozen PDFs, crops, references and protocols remain unchanged; [actual corpus admission passed](../evaluation/audits/acquisition-20261018/receipt.json).

The execution/report work must preserve all eight variants × two CPU limits × two cohorts, including failed or unexecuted cells. Strict NFC CER, secondary whitespace-collapsed CER, valid tokenization, failure coverage and paired document-cluster intervals are separate views. Three timing repetitions do not triple the statistical accuracy sample. Performance units refer to successful case responses per complete workload wall second; mixed full pages and oracle crops do not become an undifferentiated primary-page throughput claim.

## Remaining acceptance and interpretation

The 240-cell continuous pilot and the telemetry overhead pairs need complete admitted results. The new native release must finish its exact Kubernetes/reference/trace/signing chain before the 32-cell multilingual matrix can use its six-image candidate index. The SDK/Cargo Chef experiment must prove actual fresh-runner restoration, seven executable identities, complete materials, negative invalidation and net build cost; source and same-builder cache observations do not prove these properties. Automatic cache exports remain disabled except the narrowly authorized native producer path.

The complete [native profile](native-profiling.md), [math experiment](math-compaction-experiment.md) and [ablations](native-ablations.md) retain failures and unfavorable results. Two-CPU spinning-off synthetic median ratios are substantially lower, but one-CPU results are approximately neutral and the across-quota promotion rule fails. Math compaction includes a 27.5% two-CPU cohort regression. No result establishes a universally highest-accuracy setting, production throughput gain, ISA causal speedup or exhausted optimization space. The final Quarto/Seaborn matrix report and README figures must be generated from admitted new observations, rather than substituting these diagnostic studies.
