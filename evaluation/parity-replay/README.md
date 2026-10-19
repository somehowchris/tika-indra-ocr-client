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

The [2026-10-18 replay](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/evidence/2026-10-18/summary.json) completed fourteen of
fourteen cases, comparing 850 tensors with zero failures. All fourteen fresh
reference text hashes and tensor-manifest hashes equal their historical receipts.
This includes the rotated Greek input and both math settings. The corrected
native diagnostic binary is bound by SHA256; final signed-source release
admission remains separate. The unchanged harness's historical
`cpu_quota_attested: false` metadata is preserved; the new outer container and
cgroup receipts document the actual bounded replay.

The 2,108 raw files (577,648,099 bytes) remain in the shared artifact directory.
A 155,079,229-byte archive is locked by
[its SHA256 receipt](https://github.com/somehowchris/tika-indra-ocr-client/blob/1f9d4d7b1185331189daec230fa7a34343258f4e/evaluation/parity-replay/evidence/2026-10-18/raw-archive.json). Compact committed
receipts include all per-tensor comparisons, raw file hashes, source verification,
prospective amendment, original/fresh identity checks and actual container/cgroup
records. Verify their consistency with:

```sh
history=$(mktemp -d)
git archive 1f9d4d7b1185331189daec230fa7a34343258f4e \
  evaluation/parity-replay \
  runtime/crates/indra-engine/tests/evidence/2026-10-18-parity | tar -x -C "$history"
python3 -B "$history/evaluation/parity-replay/test_evidence.py"
python3 -B -O "$history/evaluation/parity-replay/test_evidence.py"
```

The historical self-test runs against that complete frozen snapshot because its
amendment binds the exact adapter source. Current replay and admission tests
remain above. Restore instructions and external-data qualifications are in the
[history index](../../docs/history.md); this Git snapshot contains receipts, not
the external raw tensor arrays.
