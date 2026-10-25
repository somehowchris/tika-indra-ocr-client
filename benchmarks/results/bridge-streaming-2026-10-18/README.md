# Bridge streaming measurement, 2026-10-18

The root Rust bridge now parses HTTP headers into fixed storage and validates
UTF-8 while writing borrowed response chunks directly to its new output file.
Only an incomplete codepoint crosses chunks in a four-byte carry array. A failed
response removes the newly created partial file; `create_new` preserves existing
files. The transport still creates one 8 KiB `BufReader`; this is not an
allocation-free claim for the entire CLI, its arguments, or the filesystem.

`cargo test --locked --test streaming_allocations` measured **zero allocations,
zero reallocations and zero deallocations** inside the changed header parser and
the entire maximum-size 4 MiB response stream, including a four-byte character
split at the transport boundary. The reader was constructed outside the measured
region. Eight unit tests and the existing fourteen real TCP protocol tests passed.

## Criterion

Rust 1.99.1 (`58cf8f9c35b355199727627d07ae34a4e2846d67`, LLVM 23.1.1),
Criterion 0.8.2, release profile (opt-level 3, thin LTO, one codegen
unit), x86_64 GNU/Linux, AMD EPYC 9V74 virtual CPU. Runs were sequential on this
shared host. `taskset -c 0` and `taskset -c 0,1` limited the eligible logical CPUs
to one and two. These are **CPU-affinity microbenchmarks**, not cgroup quota or
whole-stack OCR measurements. The algorithm is serial and has no expected
two-core scaling. Other host work was not controlled, and observed outliers are
retained in the raw samples.

Each case warmed for one second and measured for three seconds with 100 samples.
The table gives Criterion's estimated time and 95% confidence interval. The
ASCII body is 16,384 bytes; the multilingual body is 14,336 bytes, repeating Greek,
Japanese and an emoji. Output is `std::io::sink()`, so numbers exclude real
network, file I/O, and OCR. They must not be read as disk or network throughput.

| Operation | One eligible CPU | Two eligible CPUs |
| --- | ---: | ---: |
| Header parse | 303.70 ns [301.31, 306.51] | 314.49 ns [308.51, 321.57] |
| ASCII, 7-byte chunks | 16.562 µs [16.123, 17.126] | 16.709 µs [16.311, 17.263] |
| ASCII, 8 KiB chunks | 360.07 ns [346.72, 377.41] | 351.01 ns [344.25, 359.78] |
| Multilingual, 7-byte chunks | 27.687 µs [27.171, 28.474] | 28.678 µs [28.067, 29.414] |
| Multilingual, 8 KiB chunks | 9.3520 µs [8.9809, 9.8532] | 8.9175 µs [8.7445, 9.1068] |

No historical speedup is claimed: the previous whole-body implementation was
not timed in this experiment. The proved change is bounded streaming storage and
zero heap churn in the measured algorithm. Small chunk boundaries cost more
UTF-8 validation calls; keeping the existing 8 KiB transport buffer is supported
by these measurements. Optimizing these nanosecond/microsecond paths further
requires evidence that they materially affect an actual Tika request.

Reproduce from the repository root:

```sh
cargo test --locked --test streaming_allocations
taskset -c 0 cargo bench --locked --bench streaming -- --save-baseline cpu1
taskset -c 0,1 cargo bench --locked --bench streaming -- --save-baseline cpu2
```

`criterion-samples.tar.gz` contains the untouched per-case `sample.json`,
`estimates.json`, `benchmark.json`, and `tukey.json` for both saved baselines.
Measurement implementation: [streaming.rs](../../../benches/streaming.rs).
