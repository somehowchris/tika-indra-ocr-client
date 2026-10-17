# Reproduce the OCR experiments

`run.py` measures complete local process trees on one or two shared logical
CPUs. `docker_run.sh` verifies signed image digests, applies a shared CPU set
to an isolated Compose project, and records cgroup-v2 counters around a load
command. The Docker wrapper is provided for a Docker host; it was not run in
the research environment.
The environment's read-only cgroup hierarchy prevented application-container
creation; separate extracted OCI root-filesystem checks do not substitute for
Docker resource-enforcement measurements.

The runner is Python-standard-library code. Fixture generation additionally
requires the two pinned direct dependencies in `requirements-fixtures.txt`
and DejaVu fonts. The tests do not need Tika, Indra, models, or Docker.

```bash
python3 -m pip install -r benchmarks/requirements-fixtures.txt
python3 -m unittest discover -s tests -p 'test_benchmark*.py' -v
python3 benchmarks/generate_fixtures.py --output /tmp/ocr-fixtures
```

The output contains 12 image fixtures, two image-only PDFs, one native-text
PDF, exact ground truth, input/truth/font SHA-256 hashes, and a manifest.
Fixtures cover typography, small print, noise/blur, low contrast, skew, JPEG,
inversion, two-column reading order, a receipt, Latin accents, and a blank page.
They are deterministic synthetic integration cases, not representative scans.
Several renditions share one source text and therefore one `document_id`.

## Native process experiments

Build the GNU/glibc static Rust bridge and install the pinned offline backend
as described in the root documentation. Use the exact Tika 3.3.2 jar and
the pinned `tessdata_best/eng.traineddata` baseline. These commands create
path-specific plans without changing deployment configuration:

```bash
python3 benchmarks/make_local_plan.py \
  --output /tmp/ocr-plan-1 --track tika --budget 1 \
  --indra-python /absolute/path/to/indra-venv/bin/python \
  --models /absolute/path/to/models \
  --tessdata-best /absolute/path/to/tessdata-best \
  --bridge target/x86_64-unknown-linux-gnu/release/indra-tesseract \
  --jar /absolute/path/to/tika-server-standard-3.3.2.jar \
  --workers 1 --threads 1 --dpi 300

python3 -m benchmarks.run \
  --manifest /tmp/ocr-fixtures/manifest.json \
  --plan /tmp/ocr-plan-1/plan.json --output /tmp/ocr-result-1 \
  --cpus 2 --driver-cpu 8 --repeats 3 --warmup 1 \
  --concurrency 1 --max-cases 15 --timeout 120
```

Choose CPU IDs that exist on your host. At two CPUs, generate a new plan with
`--budget 2 --workers 2 --threads 1`, then run with `--cpus 2,3 --concurrency 2`.
Every Tika JVM, OCR compatibility process, backend process, and worker inherits
the same CPU set. The generator/monitor runs on a disjoint CPU. All observed
thread affinities are checked; any escape fails the experiment. Do not run
another performance experiment on those CPUs at the same time.

Use `--track raw` to compare direct Tesseract CLI invocations with the resident
Indra HTTP service on the same encoded images. Both are practical service
paths; Tesseract loads in a new process for each request and Indra's workers
remain resident, which is part of the migration's performance difference.
`--track native-auto` creates AUTO-mode Tika plans for a manifest containing
native-text controls. An Indra request observed in that control fails the run.
Do not put scanned PDFs in a no-OCR AUTO control manifest.

Also repeat end-to-end comparisons with `--tika-baseline distro` and an explicit
`--tessdata-distro` directory. The installed distro model was faster than
`tessdata_best` in the [measured comparisons](../research/benchmark-results.md),
so quantify replacement against the model actually in use. The plan hashes
both model files and identifies the active baseline; keep ratios for different
baseline families separate.

The primary two-CPU throughput profile uses two concurrent one-thread
Tesseract processes and two one-thread Indra workers. A two-thread,
single-request latency profile is a separate experiment; it should not be
silently compared with a different concurrency setting. Generate it with
`--tesseract-threads 2 --workers 1 --threads 2` and use `--concurrency 1`;
repeat with both thread settings at 1 as the matched control. `--batch 1` and
`--batch 64` control actual Indra recognition batching. `--math` enables its
additional math models and requires their verified weights.

The local Tika plan uses a 256 MiB parent heap ceiling, a 512 MiB forked JVM
ceiling, Serial GC, and `ActiveProcessorCount` equal to the experiment budget.
These are identical across both OCR engines in the local comparison. They can
differ from conservative deployment defaults; a local result does not prove
the exact deployed image/profile's performance.

## Supply a real corpus

Create a JSON manifest with paths relative to the manifest, or absolute paths:

```json
{
  "schema_version": 1,
  "synthetic": false,
  "cases": [
    {
      "id": "invoice_001_page_1",
      "document_id": "invoice_001",
      "input": "images/invoice_001_page_1.png",
      "truth": "truth/invoice_001_page_1.txt",
      "media_type": "image/png",
      "pages": 1,
      "kind": "production_invoice"
    }
  ]
}
```

Optional `input_sha256` and `truth_sha256` enforce known hashes. The runner
always records the actual hashes. For a multi-page document, transcribe all
pages in the expected reading order and supply its true `pages` count. Use
page-level image cases for page-level accuracy; a PDF-level reference cannot
by itself prove that each individual PDF page was handled correctly.

Set `--max-cases` explicitly for the intended corpus; the default is 16 and the
cap is 10,000. The runner reports which cases each engine accepted. Comparisons
pair only the common case IDs. Use separate manifests for tuning and acceptance.

## What the evidence files mean

| File | Evidence |
| --- | --- |
| `environment.json` | CPU/host visibility, exact argv/env plan, corpus/configuration hashes, selected cases |
| `requests.jsonl` | Every warmup/measured attempt, status, latency, output hash, metrics; errors remain visible |
| `summary.json` | Throughput, latency percentiles/sample caveat, unique-case accuracy, paired cluster intervals |
| `cohorts.json` | Startup, first client request, measured makespan, process observations and backend batch events |
| `<engine>-<repeat>/*.txt` | Actual text returned; no fabricated output or truth substituted on success |
| `<engine>-<repeat>/server.log` | Tika/backend diagnostics and actual tensor batch metrics |

Timing excludes offline accuracy scoring. Three timing repetitions still count
as one accuracy observation per case/configuration. Failed pages contribute
empty predictions and remain errors; they are never dropped to improve a mean.
The process exit status is nonzero for any request failure or affinity escape.

A port collision is a failed startup, never permission to benchmark another
server. `--repetition-start` permits a separately retained supplemental run of
explicitly blocked cohorts. `recover_cohorts.py` combines only cohorts with no
observed application process and no successful original request; it permits
only loopback-port changes, preserves every original attempt, and records the
replacement reason and hashes. Supplementary repetitions do not add accuracy
observations: the first measured repetition remains the accuracy sample.

The headline text score uses Unicode NFC codepoints and collapses whitespace;
case, punctuation, spelling, and reading order stay intact. Strict whitespace
scores are also present. Blank references report undefined per-page CER/WER
plus explicit false-positive counts. Micro CER/WER use ground-truth units and
can exceed 100% with many insertions. See
[`research/benchmark-methodology.md`](../research/benchmark-methodology.md)
for definitions, primary sources, and production acceptance requirements.

Native resource measurements have explicit limits. `wait4` records a command's
CPU time and largest individual-process RSS. `/proc` samples observe the whole
application tree, but can miss short-lived processes and peaks. Sum RSS counts
shared mappings repeatedly; it is not cgroup memory usage. Native affinity
constrains eligible CPUs and does not create a 1/2-CPU CFS quota. The recorded
host cgroup quota is context, not isolated application accounting.

## Signed-image Docker run

Use a Compose file that resolves every image to a registry digest and has
working healthchecks. The wrapper refuses mutable image tags, verifies the
expected certificate identity/issuer before starting containers, applies the
same CPU set to every service, retains verification evidence, and tears down
only its generated Compose project.

```bash
bash benchmarks/docker_run.sh \
  --compose /absolute/path/to/benchmark-stack.yaml \
  --output /tmp/ocr-docker-result-1 \
  --budget 1 --cpus 2 --driver-cpu 8 \
  --identity "$EXPECTED_GITHUB_WORKFLOW_IDENTITY" \
  --timeout-seconds 1200 -- \
  k6 run /absolute/path/to/arrival-rate-test.js
```

Repeat with `--budget 2 --cpus 2,3`, the same corpus, and a separate output
directory. Run on the Docker host with cgroup-v2 visibility. Each service has
a per-container quota as well as the common cpuset; the cpuset bounds combined
simultaneous execution. For a strict shared quota, provision an appropriate
parent group and additionally supply `--parent-cgroup` plus its host
`--parent-cgroup-path`; the wrapper rejects a parent whose `cpu.max` is unlimited
or exceeds the requested budget. It does not change the host's cgroup setup.
Both parent arguments are required together; the filesystem path must resolve
to an actual ancestor of every running container's cgroup. Required CPU and
memory accounting files must be readable, and a sampling failure fails the
run. `memory.peak` alone is optional for older kernels.

The wrapper deliberately overrides per-service CPU partitions with the same
eligible CPU set and an individual ceiling equal to the total budget. This
measures a shared-set benchmark profile. It does not reproduce deployment
timings when the deployment uses different per-container CPU partitions.
The controller, monitor, and load generator use the disjoint driver CPU.
Known performance environment variables are retained; other environment
values are redacted. A canonical resolved-configuration SHA-256 is also saved.

`cgroups.jsonl` records per-container CPU/throttling/memory counters and the
optional aggregate parent. Sum leaf usage, or use the parent, never both.
The external load command determines closed/open-loop behavior; preserve its
offered/completed rate, dropped iterations, errors, and raw latency samples.
