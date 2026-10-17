# Executed runtime filesystem checks

**Both real OCR stacks passed the PNG, scanned-PDF, native-PDF and recursive-metadata checks under one-CPU and two-CPU affinity on 2026-10-17.** Each stack used the pinned distroless Java runtime and Tika 3.3.2. Indra used the statically linked GNU Rust adapter and the pinned distroless Python runtime. The baseline used Tesseract 5.5.0 with the locked `tessdata_best` English model.

These checks executed the exported image filesystems through `chroot` in private user and mount namespaces. Docker successfully pulled and exported the pinned images. Starting a Docker container failed before application execution because `/sys/fs/cgroup` is read-only. **Docker builds, Compose, Kubernetes, CFS CPU quotas and container memory ceilings were not validated here.** The repository CI workflows contain those remaining deployment checks.

## Results and runtime versions

| Runtime | Executed version | Evidence |
|---|---|---|
| Java | Temurin 21.0.12.1+1 LTS | [Summary and image digests](summary.json) |
| Tika | 3.3.2 | [Indra server log](tika-rootfs.log), [baseline server log](tesseract-rootfs.log) |
| Python | Debian Python 3.13.5 | [Backend ABI, model and native-library checks](../backend-rootfs.json) |
| Indra / ONNX Runtime | 0.3.2 / 1.31.0 | [Backend checks](../backend-rootfs.json) |
| Tesseract / Leptonica | 5.5.0 / 1.84.1 | [Actual runtime output](tesseract-version.txt) |

| Stack | Affinity applied to every application thread | PNG | Scanned PDF | Native PDF | Error-free recursive metadata |
|---|---|---|---|---|---|
| Tika + Indra | CPU 4 only | Passed | Passed | Passed | Passed |
| Tika + Tesseract best | CPU 4 only | Passed | Passed | Passed | Passed |
| Tika + Indra | CPUs 4 and 5 | Passed | Passed | Passed | Passed |
| Tika + Tesseract best | CPUs 4 and 5 | Passed | Passed | Passed | Passed |

The [one-CPU record](stack-1cpu.json) and [two-CPU record](stack-2cpu.json) contain the actual process/thread observations before and after the checks. Each input was submitted to both `/tika` and `/rmeta/text`. The image-only PDF embeds the PNG losslessly without a text layer and uses page dimensions that render at the original pixel dimensions at 300 DPI. It therefore exercises PDFBox rendering, the command adapter, HTTP transport, live model inference and output-file parsing.

A separate [native-PDF check with the backend absent](native-bypass-result.json) returned the expected text with the actual adapter pointed at an unavailable endpoint. This proves the configured Tika `auto` path bypassed OCR for that native-text document. The Tesseract comparison configuration uses `ocr_only`, including for its native-PDF control.

## Execution constraints

All application processes ran as UID/GID 65532. Kernel mount records confirmed a read-only root and writable `noexec,nosuid` tmpfs: 256 MiB for each Tika root and 128 MiB for Indra. Tika retained its parent 128 MiB and forked-parser 512 MiB maximum Java heaps. These heap and tmpfs bounds do not establish that the configured container RAM limits accommodate representative documents or concurrency.

The rootfs smoke runs kept **one Indra worker with one inference thread**, recognition batch 64 and math disabled, at both affinity masks. Tesseract kept `OMP_THREAD_LIMIT=1` and `OMP_NUM_THREADS=1`. Separate final-root commands loaded and executed both the standard and math-enabled Indra profiles; their invoice input establishes model/ABI correctness, not equation recognition accuracy.

After the matched native measurements, the delivered two-CPU deployment profile was tuned to **two Indra workers, one thread each**. That template change was not rerun in these rootfs checks. Each worker loads its own model sessions and adds resident memory; the shared 2 GiB backend memory ceiling still needs representative stress validation. The two-CPU Tesseract overlay uses one OpenMP thread, following the matched measurements. The CPU ceilings remain 0.5 + 1.5 for the Tika/Indra deployment and 2.0 for the baseline.

These are correctness and dependency checks. The separate throughput/CER/WER experiments used their recorded host interpreters and engines, including Python 3.13.16 and Tesseract 5.3.4. Their results must not be presented as measurements of these distroless runtimes.

## Audit trail

[summary.json](summary.json) records the full pinned image digests, configuration IDs, runtime versions, executable hashes, source hashes and validation-only differences. [tesseract-native-files.json](tesseract-native-files.json) records every copied native-stage file, retaining loader-requested SONAMEs. [tesseract-builder.log](tesseract-builder.log) records the signed Debian snapshot installation. The normal APT privilege-drop path was verified after correcting the temporary namespace helper; no host package or network configuration was changed.

The [Docker failure log](docker-cgroup-failure.log) records the actual read-only-cgroup error. The Java and backend final roots were then assembled from the verified JAR/model bytes and the corresponding native-stage artifacts. The Tika listen ports were changed only in validation copies to 19998 and 19997, with Indra at loopback port 18081.

The Python audit scripts in this directory preserve the exact workspace paths and invocations used for this investigation. They are evidence scripts, not installation commands. Use the repository Docker/Compose/Kubernetes instructions for setup. All validation listeners and the temporary Docker daemon were stopped after the checks; [cleanup.json](cleanup.json) records the final state.
