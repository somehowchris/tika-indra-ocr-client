# Reproducible native dependency build

The Rust OCR engine needs ONNX Runtime and OpenCV, including the image codecs.
This layer builds their CPU libraries as static GNU archives in a Debian 13
builder. The existing Tika 3.3.2 baseline remains usable while the dependent
engine and service are validated.

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

The `native-evidence` scratch stage exports build evidence. It is not a serving
image. The dependent application build must still prove a runnable static ELF
with no `PT_INTERP` or `DT_NEEDED`, execute real OCR and retain its raw parity
results. GNU static distribution also requires the glibc notices and relinking
materials described in [relinking.md](relinking.md).

## Recovery correction

The interrupted hosted run failed before compilation because it required a
space after `NASM version 2.16.03`. Debian omits the optional build-date suffix.
The check now compares the parsed version field to the same exact pin. No
historical native executable or parity result validates the reconstructed
runtime. Fresh results are recorded with the producing commit and workflow.
