# Independent numerical replay

This is analysis tooling, separate from the Rust client and evaluator. It never
sends requests, runs inference, acquires data, or changes the frozen protocol.
Preparation checks small fixtures and the 43 already admitted reference texts.
An actual numerical verdict requires a later replay of the measured matrix.

`replay.py` uses arbitrary-width Myers bit vectors for unit-cost Levenshtein;
the Rust scorer uses a scalar row dynamic program. Its fixtures independently
cross-check 14,641 short Unicode-string pairs against a full-matrix oracle,
then cover token sequences, bit boundaries, analytical rates/quantiles, whole
cluster resampling, binary failure bodies, metric limits and rehashed corrupt
summaries. The algorithm follows [Myers, JACM 1999](https://doi.org/10.1145/316542.316550),
adapted for global distance rather than substring matching. SplitMix64 constants
and wrapping arithmetic agree with [Vigna's reference generator](https://prng.di.unimi.it/splitmix64.c);
a known-vector fixture and forced rejection draw check the independent port.

Unicode versions are deliberately distinct:

- `unicode-normalization 0.1.24` and `unicode-general-category 1.1.0` use Unicode
  16.0.0. Python's independent NFC/category provider must report exactly 16.0.0;
  [Python 3.14 documents that database](https://docs.python.org/3.14/library/unicodedata.html).
- The producing `rustc 1.99.1` reports Unicode 17.0.0. Its standalone
  `rust-unicode-probe.rs` enumerates every scalar through `char::is_whitespace`.
  The resulting 25 scalars match [Unicode 17 White_Space](https://www.unicode.org/Public/17.0.0/ucd/PropList.txt).
  The checker uses that explicit set; Python `isspace` additionally recognizes
  U+001C–U+001F and would change the protocol. A different recorded Rust Unicode
  version fails replay until its actual whitespace contract is reviewed.

The checker verifies each consumed reference, journal, response, score and
comparison hash. Frozen manifests/protocols must equal the reviewer-selected
application producer's exact Git bytes. It records that application identity
separately from its own committed source and executable Python identity. It
reads only bounded text records; it does **not** repeat OCI signature, image,
input-image, model, parent-cgroup, or full outer inventory admission. Those
remain prerequisites in the existing release/study validators.

Without `--evidence`, it independently reconstructs the English NFC **Letter**
denominators, including conservative unknown-English declarations, and records
all 43 reference hashes. Whitespace/punctuation CER units are not substitutes
for Letter counts. It also reports an all-success upper bound on the requested
bootstrap cluster draws; failed or ineligible cases can reduce that work.

```sh
python3 -B -m unittest discover -s evaluation/independent -p 'test_*.py' -v
python3 -B -O -m unittest discover -s evaluation/independent -p 'test_*.py' -v

# Use a clean committed checker; this command writes a new receipt.
python3 -I -B evaluation/independent/replay.py \
  --repository . \
  --source-sha 92ba8d549e32f6a12b9e6154b238114cf754c10b \
  --main-references /absolute/acquired/main \
  --laboratory-references /absolute/acquired/laboratory \
  --output /absolute/new/reference-only.json
```

Add `--evidence /absolute/combined` to replay an aggregated `study-index.json`.
The [replay workflow](../../.github/workflows/ocr-replay.yml) does this on a
hosted `ubuntu-24.04` runner with Python 3.14.7. The evaluation workflow runs it
after every aggregate, and the OCR evaluator workflow dispatches it for a named
`ocr-evaluation-SHA-RUN-ATTEMPT-combined` artifact or, by default, the newest
retained one. [`scripts/replay-ocr-evaluation.sh`](../../scripts/replay-ocr-evaluation.sh)
first admits only evidence sealed by a Signed release run of this repository at
the named source and checks the evidence's own `SHA256SUMS`. It then acquires
the references with the evaluator bytes the admitted shards executed, through
the matrix controller's own acquisition path (`fetch` and `prepare` for the main
cohort, committed laboratory references), and runs the checker under a
1,800 CPU-second, 4 GiB address-space and 2,700 s wall bound. Stdout, stderr,
exit status, the exact command and the JSON receipt are uploaded even on
failure. No interval is silently subsampled to meet a deadline.

Actual replay independently reconstructs every available first-repetition case
metric, all six reporting axes, all repetition timing counts and descriptive
type-7 quantiles, completed-workload throughput, and every saved paired report.
Every assigned cell and all 60 comparison statuses remain in the result,
including missing, failed and skipped records. A numerical match does not promote
a failed driver or an untrusted artifact. Structural errors and all arithmetic
differences remain explicit; the command returns nonzero. Integer counts, seeds
and identities require exact types/values. Derived JSON float fields use relative
tolerance 1e-12 and absolute tolerance 1e-14; this never relaxes edit or sample
counts. Non-finite literals, exponent overflow and out-of-range consumed Rust
integer/error-label fields are rejected before arithmetic.

First-attempt failure remains failure despite later success. A successful body
with invalid UTF-8 is a hard scoring error; failed bodies remain byte-hashed
without text decoding. A first-attempt metric limit excludes every metric for
that case but remains an HTTP success for transport timing. Empty references
retain insertion counts with undefined rates, and rates may exceed 100%.
Intervals resample sorted whole document clusters, reset the independent RNG
for each metric, use the frozen length-prefixed stratum seeds and every requested
draw, and stay undefined for fewer than two eligible clusters or any undefined
draw. Laboratory draws stay zero and its cases never join the main cohort.
