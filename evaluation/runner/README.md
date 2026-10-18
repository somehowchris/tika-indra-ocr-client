# Reproduce and evaluate the frozen OCR study

`indra-evaluate` prepares the public corpus, checks its admission rules, runs one
prespecified Tika arm, and scores its retained responses. The production OCR
implementation and this evaluator are Rust. The upstream Python image is an
explicit comparison arm; the separate Quarto report uses the requested Seaborn
visualization toolchain.

The [protocol](../protocol.json) defines the eight primary variants, one/two-CPU
budgets, repeated request order and scoring policy. The
[manifest](../corpus-frozen.json) freezes 33 primary pages from 19 document
clusters and eight separate crop challenges. These are diagnostic cases, not a
representative sample of production documents. Modern laboratory reports and
contemporary scientific manuscripts are absent; historical biology pages do
not fill that gap. Source rights, reference provenance and editorial caveats
remain in the manifest and [audits](../audits/).

## Build and prepare

Use the pinned Rust toolchain from this repository. Build with the checked-in
lock; the release profile enables optimization, thin LTO, one codegen unit,
abort-on-panic and overflow checks.

```sh
cargo build --locked --release --manifest-path evaluation/runner/Cargo.toml
OCR_EVALUATOR="$PWD/evaluation/runner/target/release/indra-evaluate"
OCR_CORPUS="$PWD/corpus-data"

"$OCR_EVALUATOR" fetch \
  --lock evaluation/acquisition/frozen-downloads.json --root "$OCR_CORPUS"
"$OCR_EVALUATOR" prepare \
  --manifest evaluation/corpus-frozen.json --root "$OCR_CORPUS" \
  --locks evaluation/acquisition/locks
"$OCR_EVALUATOR" verify \
  --manifest evaluation/corpus-frozen.json --root "$OCR_CORPUS"
```

Preparation uses the pinned Rust image/XML/ZIP dependencies. Legal PDF
preparation requires `pdftoppm` and `pdftotext`; the exact reconstruction was
verified with **Poppler 26.05.0**. A different renderer or encoder is accepted
only if every frozen size and SHA-256 still matches. The tool does not replace
sources, resample images, change a reference, or select cases using OCR output.
Publisher URLs can change or reject automated clients; the
[publisher route record](../acquisition/publisher-routes.json) documents the
legitimate alternate distributions that returned the same frozen PDF bytes.

Outputs are promoted without overwriting an existing different file. A
conflicting or mismatched candidate remains available for inspection.
Re-running successful preparation verifies and reuses the existing artifacts.
External PDF tools receive preopened output handles and a finite deadline.

Admission checks all 82 input/reference hashes, accepted source metadata,
source/reference audit fields, unique primary-page identities, document
clusters and declared tokenizers. English must remain strictly below 10% of
primary pages, clusters **and NFC Unicode Letter characters**. Unknown English
status contributes all of the case's letters to the conservative numerator.
The frozen primary counts are 2/33 pages, 1/19 clusters and 51/87,515 letters.
Unicode `is_alphabetic` is not the definition used for this denominator.

Acquire Tesseract weights with the separate lock, outside measurement:

```sh
"$OCR_EVALUATOR" fetch \
  --lock evaluation/acquisition/tesseract-models.json --root "$OCR_CORPUS"
```

The selected mount is the `tesseract/fast` or `tesseract/best` leaf, including
its locked `configs/txt`. Indra mounts the model revision directory verified
by the service's own model manifest. Model acquisition is never part of a
measured OCR request.

## Run a controlled arm

Use a Linux/amd64 host with local Docker, its systemd cgroup driver, unified
cgroup v2 and an ordinary 8 MiB main-thread stack. The driver requires a
precreated, empty systemd slice with exactly one or two effective CPUs,
matching CPU quota, 3 GiB aggregate memory and zero swap. A stricter CPU or
memory limit in an ancestor is rejected. The workflow owns slice creation;
the driver verifies the real systemd-to-cgroup mapping before starting an arm.
Create a fresh, uniquely named parent slice for every arm. An empty reused
cgroup still retains its lifetime memory high-water counter; emptiness alone
does not make its peak specific to the next arm. The driver requires initial
CPU usage, current/peak memory and live/dying child-group counts to be zero,
then records the parent device/inode and rejects replacement during the arm.
The host must expose `memory.peak` for this admission check.

Every Tika, OCR and load-generator process is a descendant of that parent.
Each container also has a 3 GiB leaf limit; the **shared parent** is the binding
memory budget. This avoids imposing a smaller private memory ceiling on the
Tesseract stack. Tika uses the same gateway and JVM policy in every arm.
The driver does not override it through `JAVA_TOOL_OPTIONS`.

Images must already exist locally and be identified by
`repository@sha256:...` or their full local `sha256:...` image ID. Tags are
rejected. The driver inspects the requested digest, creates containers from
the resolved immutable ID with `--pull=never`, and checks their resulting
image, user, mounts, namespace, restart policy and resource settings. Image
signature verification belongs to the image acquisition/release workflow;
an immutable ID alone is not a signature.

Example native text arm, after the workflow has prepared the slice and pinned
images:

```sh
"$OCR_EVALUATOR" docker \
  --arm native --label rust-text-b64 --cpus 2 --threads 2 \
  --batch-size 64 --repetitions 3 \
  --tika-image "$TIKA_IMAGE_DIGEST" --backend-image "$NATIVE_IMAGE_DIGEST" \
  --evaluator "$OCR_EVALUATOR" --tika-config deploy/evaluation/tika-indra.xml \
  --models "$INDRA_MODEL_REVISION_DIRECTORY" \
  --corpus "$OCR_CORPUS" --manifest evaluation/corpus-frozen.json \
  --protocol evaluation/protocol.json --output "$PWD/results/rust-text-b64-cpu2" \
  --parent-slice "$OCR_PARENT_SLICE" --parent-cgroup "$OCR_PARENT_CGROUP" \
  --affinity "$OCR_CPU_AFFINITY"
```

The output directory must be new. Paths must be UTF-8 and exclude whitespace
and Docker/systemd property separators. Commands use separate argument values;
no shell evaluates them. Run `"$OCR_EVALUATOR" docker --help` for the complete
argument list. Tesseract additionally requires `--tesseract-models-lock` and
`--tesseract-psm`; it has no backend image. Math variants add `--math`.
Primary variant settings must agree with the frozen protocol. An undeclared
label or an improvised batch/thread/PSM combination fails before any requests.
The protocol also requires `tika_indra_config_sha256` and
`tika_tesseract_config_sha256`. Native and Python Indra use the same first XML
identity; Tesseract uses `deploy/evaluation/tika-tesseract.xml` and the second
identity. The driver verifies the hash of the exact retained XML bytes mounted
into Tika before it creates any container.

The containers use read-only roots and input mounts, an explicit writable
`/tmp`, user 65532, dropped capabilities, no new privileges, PID limits and no
restart policy. Only the Tika gateway is published, on an allocated IPv4
loopback port. Its OCR sidecar shares the gateway's network namespace and
listens on loopback. Image-provided Docker healthchecks are disabled uniformly;
the driver explicitly checks readiness before measurement. Readiness polling
issues no corpus OCR request.
Both Tika XML files impose the same 60-second OCR subprocess timeout. Native
and Python Indra receive explicit 65-second internal request deadlines; the
bridge retains its 70-second deadline. Late work stays inside the measured
parent until cleanup. The load generator's whole HTTP request limit is 120 seconds.

The load generator runs through `systemd-run` inside the same parent, with its
own finite runtime limit and read-only corpus. It independently verifies the
live gateway/OCR process IDs, start identities, cgroup ancestry, quota, affinity
and memory limits at the beginning and end. One warmup request precedes three
serial repetitions of every frozen case. A timeout, HTTP error, invalid UTF-8
body or oversized response remains a failed attempt; it is not retried.

Shutdown first stops any remaining load-generator unit. Tika then loses
readiness and drains accepted requests while OCR remains available. After
Tika stops, OCR stops. The driver retains stopped-container inspections and
logs before removal, including errors and OOM status. Cleanup uses unique
ownership labels and a separate command/output reserve. If the daemon or
ownership checks prevent confirmation of Tika shutdown, the driver reports
the failure and retains the sidecar rather than interrupting potentially
accepted work. The outer workflow must also retain artifacts on failure.

This behavior follows Docker's [container run](https://docs.docker.com/reference/cli/docker/container/run/)
and [stop](https://docs.docker.com/reference/cli/docker/container/stop/) contracts,
and systemd's [transient service](https://www.freedesktop.org/software/systemd/man/latest/systemd-run.html)
and [resource control](https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html)
interfaces. A Docker Desktop VM or an unconstrained local process does not
satisfy this experiment's host admission requirements.

## Score after measurement

```sh
"$OCR_EVALUATOR" score \
  --manifest evaluation/corpus-frozen.json --root "$OCR_CORPUS" \
  --run results/rust-text-b64-cpu2/run --output results/rust-text-b64-cpu2/score

"$OCR_EVALUATOR" compare \
  --left results/python-text-b64-cpu2/score/scores.json \
  --right results/rust-text-b64-cpu2/score/scores.json \
  --output results/python-vs-rust-cpu2 --seed 20261017 --resamples 10000
```

Scoring requires the exact manifest bytes recorded by the run, rechecks its
assets, and verifies each retained response's size and SHA-256. Failed binary
responses are hashed without attempting UTF-8 decoding. An interrupted final
JSONL record, including a cut inside a UTF-8 scalar, is recorded as an
unscored tail. An incomplete run can produce an honest partial score report;
it cannot enter a paired comparison.

Accuracy always selects **repetition 1** for each case. A later successful
request never replaces a failed first attempt. Repetitions measure performance
and output stability, not extra independent accuracy samples. Successful
response hashes that vary across repetitions and mixtures of success/failure
are reported separately.

| Measure | Definition |
| --- | --- |
| Primary CER | Unit-cost Levenshtein distance on NFC Unicode scalars; preserve case, punctuation and whitespace |
| Secondary CER | The same metric after collapsing and trimming Unicode whitespace only |
| WER | NFC whitespace tokens, only for cases explicitly declaring that tokenizer; no fabricated CJK word score |
| Micro CER | Sum of edit counts divided by sum of reference units |
| Case-macro CER | Mean of defined per-case rates; retain empty-reference insertion counts separately |
| Comparison | Right minus left, on joint first-attempt successes; failures and excluded cases remain enumerated |
| Uncertainty | Paired resampling of complete source-document clusters, fixed seed and 10,000 resamples |

Intervals are omitted with fewer than two eligible document clusters, or when
a bootstrap ratio is undefined. Primary and challenge cohorts remain
separate, as do language, script, domain, appearance and reference-quality
strata. Two Chinese crops with unresolved page identity retain their shared
document cluster. Repeated requests and multiple crops are not promoted to
independent documents. The intervals describe this small selected corpus and
are not multiplicity-adjusted claims of general superiority.

## Read the evidence

| Artifact | Purpose |
| --- | --- |
| `configuration/`, `input-identities.json` | Exact protocol, corpus, Tika configuration, optional Tesseract lock and evaluator identity |
| `models-before.json`, `models-after.json` | Bounded mounted-file size/hash inventories and mutation check |
| `parent-before.json`, `parent-after.json`, `hardware/` | Parent/ancestor limits, effective affinity, CPU flags and kernel context |
| `parent-freshness.json` | Zero prior CPU/memory activity and no live/dying child cgroups before the arm |
| `parent-counters-after.json` | Whole-parent CPU and charged-memory counters after service shutdown, including startup and any late work |
| `commands/` | Exact argv/environment changes, stdout, stderr, elapsed control time and termination status |
| `readiness.json`, `driver-result.json` | Startup/readiness context and execution, cleanup and integrity outcome |
| `run/requests.jsonl`, raw response files | Every attempt, response hash, latency, repetition boundary and live resource attestations |
| `score/scores.json`, `score/cases.csv` | Per-case outcome, first-attempt metrics, stability and cohort/stratum summaries |
| `score/attempts.csv`, `score/performance.json` | Every non-warmup timing observation, separate success/failure distributions and complete-workload throughput |
| `comparison.json`, `pairs.csv`, `paired.csv` | Matched case identities, cluster counts, exclusions and bootstrap estimates |

Successful HTTP latency runs from sending the request through reading its complete
response body. Failed attempts record elapsed time until the observed error,
not an inferred OCR completion time. UTF-8 validation, response hashing, file
persistence and journal writes follow that timer; repetition wall time includes
those intervening operations.

The timing export preserves all non-warmup repetitions and their case, document,
cohort and language/script/domain/appearance/reference-quality labels. It reports
successful-response latency separately from failed-attempt termination time.
HTTP success requires a 2xx status and no recorded error; a later offline metric
limit does not change that timing classification. Primary and challenge latency
distributions remain separate. Their median, p95 and p99 use type 7 linear
interpolation at rank `(n - 1) × p`. Empty distributions have undefined quantiles.
These are descriptive summaries of correlated repeated observations; a small
p99 sample does not establish a production tail-latency guarantee.

Whole-workload throughput divides completed successful frozen case responses by
the sum of repetition wall times, including failures and the work between
requests. Cases can be full pages or oracle crops, and repetitions contribute
additional responses rather than independent pages. The field is named
`successful_case_responses_per_complete_workload_wall_second`. It is reported
only when every expected attempt and repetition boundary is present in a
complete journal. The denominator covers the entire frozen sequence, including
both cohorts when selected, so the evaluator does not invent separate cohort
wall-clock throughput. Interrupted runs retain observed latencies and missing
counts, with throughput left undefined. Accuracy and timing exports carry the
same manifest and source-journal SHA-256 and byte counts.

Parent CPU deltas start after corpus admission and before HTTP client/buffer
setup, span warmup and all repetitions, and end before final journal sync and
service shutdown. The driver's separate final parent counters extend through
service shutdown and therefore include startup, warmup, measurement and late
internal work. Charged-memory high-water counters can include model loading and
startup; they are not labeled steady-state-only memory. Keep the earlier synthetic signed baseline separate
from this corpus and its whole-stack accounting policy.

The model inventory reads every model file before container startup, warming
the host file cache. Readiness timings therefore measure process and model
initialization with a warmed model-file cache; they do not measure storage-cold
startup. Cgroup memory reports charged memory, not summed process RSS. Shared
file pages can remain charged to the cgroup that first instantiated them, even
when another cgroup later uses them; see the kernel's
[memory ownership rules](https://docs.kernel.org/admin-guide/cgroup-v2.html#memory-ownership).
Retain this qualification when comparing the model families' memory usage.

The metric workspace bounds each NFC text at 32,768 scalars and each token
sequence at 16,384 words. Exceeding a bound produces a metric-limit outcome,
never truncation. Command streams retain at most 32 MiB per stream, with a
256 MiB control budget and 128 MiB cleanup reserve; exceeding a limit is an
explicit failure. Full container log retention is also constrained by the
recorded Docker rotation policy.

`cargo test --locked` exercises analytical edit-distance/bootstrap/quantile fixtures,
adversarial Unicode, first-attempt failure selection, interrupted journals,
success/failure timing separation, serial wall-time bounds, source/model/protocol
admission, fresh-parent counters, exact Tika XML binding, safe file promotion and
subprocess deadlines.
The full frozen identity fixture proves scorer plumbing by construction; its
zero edit distances are not OCR results. Real container readiness, shutdown
and bounded-hardware measurements require the hosted acceptance workflow.
