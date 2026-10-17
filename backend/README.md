# Indra image backend

This service keeps Indra OCR loaded in one or two worker processes. Tika sends rendered images through the repository's static GNU/glibc Rust bridge. The backend listens on `127.0.0.1:8081`; Docker Compose and Kubernetes share the network namespace with Tika. The Python, ONNX Runtime and JVM runtimes use distroless images and native shared libraries.

## Run locally without a container

Use Python 3.13. Commands below run from the repository root. Install into an isolated environment and fetch the exact checked model snapshot once:

```sh
python3.13 -m venv .venv
.venv/bin/pip install --require-hashes --only-binary=:all: -r backend/requirements.lock
.venv/bin/python backend/fetch_models.py --output models
INDRA_MODEL_DIR="$PWD/models" .venv/bin/python backend/server.py
```

The model directory must contain ordinary files with the hashes in `models.lock.json`. Startup rejects missing, modified, symlinked or additional unlocked ONNX files. The runtime never calls the model downloader. The build fetches both standard and math models so changing the math profile does not require network access.

In another terminal:

```sh
curl --fail http://127.0.0.1:8081/readyz
curl --fail --data-binary @backend/tests/data/smoke.png http://127.0.0.1:8081/ocr
```

The smoke image contains `INVOICE 12345`.

## Image build

```sh
docker build -f backend/Dockerfile -t tika-indra-backend:local .
```

The pinned Debian 13 builder creates a Debian Python 3.13 virtual environment. The final image is the pinned `gcr.io/distroless/python3-debian13:nonroot` image. The build copies the required `libstdc++.so.6` and `libgcc_s.so.1` into `/opt/native`, preserving their loader names. Exec-form build checks import native libraries and perform actual OCR with both model profiles using the final image's interpreter. No shell or package manager is added to the final image. The Dockerfile uses the Docker engine's bundled frontend.

Run the complete stack using the root deployment instructions. To test the backend alone in a container, deliberately override its binding with `-e INDRA_HOST=0.0.0.0` and publish only to host loopback, for example `-p 127.0.0.1:8081:8081`. The shared-namespace deployments keep the default loopback binding.

## Contract and limits

| Request | Result |
|---|---|
| `GET /healthz` | `200`, `alive\n`; listener process liveness |
| `GET /readyz` | `200`, `ready\n` when every model worker is alive and ready; otherwise `503` |
| `POST /ocr` | Raw PNG, JPEG or TIFF; required, single decimal `Content-Length`; UTF-8 plain text |

Every application response includes `Content-Length`, `Connection: close`, and `Content-Type: text/plain; charset=utf-8`. Chunked transfer, compressed HTTP bodies and `Expect: 100-continue` are rejected. PDF input belongs at Tika. TIFF frames are processed in order; text between frames is separated by `\n\f\n`.

The service bounds input to 20 MiB, decoded pixels to 25 million across at most 16 frames, and output to 4 MiB. Pillow inspects dimensions and frame counts before OpenCV decompresses pixels; OpenCV also receives its own pixel limit. Oversized input or output returns `413`; invalid images return a non-success response. Output is never silently truncated. A valid blank page may correctly return an empty `200` response.

One worker is the default. The default admission limit is one active request plus two waiting requests, with two seconds maximum queue wait. Additional requests receive `503`. Handler threads and the socket backlog are separately bounded. Health probes have spare handler capacity and do not wait for a model worker.

The parent gives each request a 60-second processing deadline. A timed-out native call causes child termination and `504`; a replacement loads and warms independently while readiness is false. If replacement fails, or an idle ready child dies, the supervisor exits nonzero. Docker/Kubernetes process restart policy then applies; readiness failure alone would not restart a process.

## Configuration

| Variable | Default | Meaning |
|---|---:|---|
| `INDRA_HOST` | `127.0.0.1` | Bind address |
| `INDRA_PORT` | `8081` | Bind port |
| `INDRA_MODEL_DIR` | `/models` | Verified local model files |
| `INDRA_MODEL_MANIFEST` | Next to `server.py` | Exact model manifest |
| `INDRA_WORKERS` | `1` | Persistent processes, allowed `1` or `2` |
| `INDRA_THREADS` | `1` | ORT intra-op and OpenCV threads, allowed `1` or `2` |
| `INDRA_MATH` | `0` | `1` enables the optional math classifier and recognizer |
| `INDRA_RECOGNITION_BATCH` | `64` | Maximum recognition line batch, `1`–`64` |
| `INDRA_BATCH_METRICS` | `0` | `1` records actual ORT recognition tensor shapes |
| `INDRA_QUEUE_CAPACITY` | `2` | Waiting admissions in addition to workers, `0`–`16` |
| `INDRA_QUEUE_TIMEOUT_SECONDS` | `2` | Maximum wait for an available worker |
| `INDRA_READ_TIMEOUT_SECONDS` | `10` | Body read deadline and socket inactivity timeout |
| `INDRA_REQUEST_TIMEOUT_SECONDS` | `60` | Overall request processing deadline after headers |
| `INDRA_STARTUP_TIMEOUT_SECONDS` | `120` | Initial/replacement worker loading deadline |
| `INDRA_MAX_REQUEST_BYTES` | `20971520` | May reduce, never increase beyond 20 MiB |
| `INDRA_MAX_PIXELS` | `25000000` | May reduce, never increase beyond 25 million |
| `INDRA_MAX_FRAMES` | `16` | May reduce, never increase beyond 16 |
| `INDRA_MAX_RESPONSE_BYTES` | `4194304` | May reduce, never increase beyond 4 MiB |

`workers × threads` cannot exceed two. This check prevents an accidental oversized configuration; it does not implement CPU isolation. Use the supplied container budgets or an inherited CPU affinity for the complete Tika + bridge + backend process tree. For a whole-stack two-vCPU deployment, two backend workers leave less CPU time for Tika than one backend worker. Choose between those profiles using the measured full-stack workload.

ORT runs the exact standard `rec.onnx` model with `rec_int8=False` and only `CPUExecutionProvider`. All six published standard models are required. Each worker sets OpenMP, BLAS and OpenCV thread limits before importing native code. Upstream uses one inter-op thread and disables the CPU allocation arena. No provider, precision or language-model substitution occurs automatically.

## What batching means

Indra batches detected line crops within each page. Its upstream `width_batches` sorts by crop width, uses `max(width, 64)` for a ratio limit of 1.3, limits count to the configured recognition batch, and checks `count × max(width, 64) ≤ 40,960` pixel columns. The 64-pixel floor affects grouping only. Recognition crops are 32 pixels high and their actual maximum width is rounded up to a multiple of eight, so the grouping budget is a proxy for padded work, not an exact final tensor-area bound. Chinese and non-Chinese recognition runs are separate. See the pinned [preprocessing source](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/rec_preproc.py) and [recognition pipeline](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/pipeline.py).

The service preserves this behavior. HTTP concurrency and worker processes do not combine crops from different requests. The configured recognition batch also reaches the script classifier through upstream's method argument. Math classification and decoding retain their independent upstream batch defaults. Math decoding uses 64-pixel-high crops and a wider grouping policy; see [math.py](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py).

With `INDRA_BATCH_METRICS=1`, JSON log records include stage, actual batch count, tensor height, padded width, valid pixel columns and call count. This observation layer forwards the same tensors into ORT. Its overhead is included in instrumented timings; use an uninstrumented run for the final throughput comparison.

The implementation makes necessary copies explicitly: bounded HTTP buffering, process IPC, image decoding, crop resizing/padding, and UTF-8 response serialization. The NumPy view of compressed bytes and the optional session meter avoid additional tensor copies. There is no claim of an end-to-end zero-copy pipeline.

## Verify

```sh
# Invariants plus HTTP behavior against a separately running backend:
INDRA_TEST_URL=http://127.0.0.1:8081 \
  .venv/bin/python -m unittest discover -s backend -p 'test_server.py' -v

# Test-owned child failures, isolated model directory entries, real restarts:
INDRA_TEST_MODEL_DIR="$PWD/models" \
  .venv/bin/python -m unittest discover -s backend -p 'test_recovery.py' -v

# The same actual native/OCR check executed by the final-image build:
INDRA_MODEL_DIR="$PWD/models" .venv/bin/python backend/image_smoke.py
INDRA_MODEL_DIR="$PWD/models" .venv/bin/python backend/image_smoke.py --math
```

These are correctness and recovery checks on synthetic inputs. Accuracy and 1/2-vCPU performance evidence belong in the root benchmark report; a smoke test is not an accuracy benchmark.

The native performance measurements used the isolated CPython 3.13.16 environment. The exact pinned Debian 13 distroless root filesystem was separately verified with its CPython 3.13.5 interpreter, the same locked packages, UID/GID 65532, a read-only root and a 128 MiB `noexec,nosuid` temporary filesystem. Both exact final-stage OCR smoke commands passed there. The math-enabled smoke uses an invoice image: it verifies the math profile loads and the pipeline runs, rather than measuring equation recognition quality. See [runtime evidence](../validation/backend-rootfs.json). Exporting and executing the root filesystem does not establish Docker execution, a CFS quota, or Kubernetes behavior.
