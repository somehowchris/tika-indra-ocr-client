# Reproducible native dependency build

The Rust OCR engine needs ONNX Runtime and OpenCV, including the image codecs.
This layer builds their CPU libraries as static GNU archives in a Debian 13
builder. The pinned native dependency build completed successfully in
[run 39086092581](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39086092581).
The reconstructed engine subsequently passed a real static GNU link, scratch
TLS model acquisition/reuse and native acceptance under both actual one-CPU and
two-CPU quotas. The [engine evidence](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/runtime/crates/indra-engine/tests/evidence/2026-10-18-parity/README.md)
records the exact producing revisions, artifact hashes and corrected reruns.

## Exact inputs

`scripts/native-build-tools.lock` pins the source commits, Intel IPP archive,
build-tool versions and wheel hashes. `docker/native.Dockerfile` pins the Rust
builder digest and Debian package snapshot. ONNX Runtime's pinned `deps.txt`
identifies its transitive native inputs. The build verifies the resulting
CMake configuration and records every archive hash and third-party notice.

| Input | Pin |
| --- | --- |
| ONNX Runtime | [1.31.0, `402979ac`](https://github.com/microsoft/onnxruntime/tree/402979ac1bef89dcd55add41eaa7277c407599eb) |
| OpenCV | [5.0.0, `40738fb1`](https://github.com/opencv/opencv/tree/40738fb16ceddb5fb3fea747585f7ce6abb0605b) |
| Rust / libc family | 1.99.1, Debian GNU/glibc |
| CPU policy | Portable SSE3 baseline; OpenCV runtime dispatch through AVX512_SKX |

The build disables OpenCV's internal tracing and ONNX Runtime telemetry,
Python bindings, GPU providers, OpenMP and TBB. JPEG SIMD and IPP are verified
as enabled. IPP is an upstream prebuilt component; the native closure must not
be described as entirely source-built. Application observability is provided
by the service's own tracing integration in a dependent PR.

## Run

```sh
docker buildx build --platform linux/amd64 \
  --file docker/native.Dockerfile --target native-evidence \
  --build-arg BUILD_CPUSET=0,1 \
  --output type=local,dest=native-evidence .
```

Select two CPU IDs that are allowed on the builder. The scripts compile the
libraries sequentially, with two compilation jobs. This is a build resource
policy; it is not a one/two-vCPU OCR benchmark. The `Native GNU runtime` action
retains its execution scope and evidence even when a later step fails.

The `native-evidence` scratch stage exports build evidence. `engine-acceptance`
links and executes the image OCR gate; `server` is the native HTTP serving
image. The application build rejects a runnable ELF with `PT_INTERP` or
`DT_NEEDED`. Each explicit rejection must exit nonzero; a standalone negated
`grep` under `bash -e` would not enforce that gate.

The accepted engine artifact from
[run 39092252965](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39092252965)
passed 23 unit tests and three native acceptance tests at both quotas, including
distinct TIFF pages and real optional math decoding. The frozen comparison
then checked exact text for 14 selected implementation cases and the
prespecified comparisons for 850 intermediate tensors. Those are historical
results for the identified artifact; changes to batching, profiling or the
service lifecycle need their own fresh gates.

GNU static distribution includes the glibc source packages, notices, vendored
Rust source and native link inputs described in
[runtime-relinking.md](runtime-relinking.md). The diagnostic profiling export
retains the full material bundle and binds it to the actual profiling binary's
SHA-256. It does not rely on the release server's different executable identity.

## Profiles and measured optimization

`release` uses optimization level three, thin LTO, one codegen unit, checked
integer overflow and stripped symbols. `profiling` inherits those optimization
settings and retains Rust DWARF line tables and symbols. `maxperf` changes thin
LTO to fat LTO; its name is not evidence of a faster result. No unsupported
`target-cpu=native` tuning is applied to distributable images.

The configured OpenCV dispatch set establishes which kernels can be selected;
it does not establish which instruction path ran or its causal speed benefit.
[The profiling protocol](native-profiling.md) records the actual CPU, kernel,
sampling capabilities, symbols, cgroup limits and successful timing results.
Hardware-counter and ISA-effect claims require dedicated supported
measurements. Source compilation or an available AVX flag is insufficient.

## Recovery correction

The interrupted hosted run failed before compilation because it required a
space after `NASM version 2.16.03`. Debian omits the optional build-date suffix.
The check compares the parsed version field to the same exact pin. The later
successful build and engine evidence above supersede that failed attempt.
Another real service gate exposed stack exhaustion in a default Rust worker
thread; the engine embedding reserves an explicit 8 MiB stack. The service's
fixed executable must pass its real process and deployment lifecycle gates;
standalone main-thread engine success alone does not prove them.
