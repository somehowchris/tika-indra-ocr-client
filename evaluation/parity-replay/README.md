# Replay the preserved fourteen-case implementation diagnostics

This audit adapter relocates the original inputs and executes the unchanged,
hash-checked reference hooks and native comparison harness. It does not edit the
historical evidence or replace OCR algorithms. Fresh reference arrays are new
evidence: the original raw arrays were deleted. Their text and manifest hashes
are compared with the historical receipts, without claiming their recovery.

The adapter requires the exact original fourteen input byte streams, model
artifacts and upstream Python source bytes from commit
`3d773aeedc5d79d1d0a2a04007a9075cd71a59f4`. The prior backend image is pinned by
digest in `replay.py`. Its Python interpreter differs from the historical host;
the new interpreter hash, complete package inventory and source verification
are recorded. This is a correctness diagnostic, never a performance or quality
measurement.

Run the dependency-free admission tests first:

```sh
python3 -B evaluation/parity-replay/test_replay.py
python3 -B -O evaluation/parity-replay/test_replay.py
```

The four explicit phases use exclusive output files and retain failures:

1. `stage --repo REPO --corpus CORPUS --inputs INPUTS` copies and verifies thirteen
   available inputs. Only the clockwise Greek rotation remains unstaged.
2. `prepare --repo /repo --inputs /inputs --upstream /upstream --output /replay/evidence`
   runs inside the pinned reference image, verifies installed source against all
   sixteen independently downloaded exact-commit upstream files, checks every
   model, recreates the rotation using pinned OpenCV and verifies its frozen
   SHA256. It writes the prospective source/binary/environment/path amendment
   before any OCR predictions.
3. `reference` with the same arguments invokes the unchanged original reference
   hooks for all fourteen cases. Only complete success creates the new comparison
   protocol, binding fresh reference hashes before any native predictions.
4. `native` with the same arguments executes the actual `/native-ocr` binary via
   the unchanged comparison harness, including captured/uncaptured agreement,
   exact UTF-8 and preprocessing, exact tensor sequence/shapes/dtypes and all
   original floating-point tolerances. Every mismatch remains in the output.

Each container must have one CFS CPU, 3 GiB memory, zero swap, one allowed logical
CPU affinity, no network, a read-only filesystem and bounded temporary storage.
The adapter rejects a different CFS or memory limit. Bind the source and upstream
read-only; inputs are writable only for preparation, then read-only. Bind the
native diagnostic binary read-only at `/native-ocr`. Preserve `docker inspect`
receipts and stdout/stderr for every phase, including failed preparation. The
adapter records cgroup counters before/after each arm; these do not establish a
performance result. Freeze and commit this adapter before the preparation phase.
Do not overwrite or reuse an attempt directory after any failure.
