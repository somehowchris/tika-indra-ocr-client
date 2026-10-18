# Actual production HTTP reference outputs

All28 selected HTTP responses matched the frozen upstream reference UTF-8 bytes:
14 cases at1CPU and the same14 at2CPU, including form-feed page separators and
both math modes. The actual server is `c9cfe911...` in immutable assembled image
`cbc65a...`. Client ELF `7f6e2fd5...` was built from697fec4; the executed controller
was304ccf4. Their distinct identities and client-source equality mapping remain
recorded. Every owned container exited0 without OOM and was removed.

The original controller exited1 after all requests and cleanup: its summary glob
also included four OCI helper `result.json` files. That original status remains
unchanged. The derivative acceptance selects only the original frozen case paths,
rechecks all28 body/wire hashes against frozen reference hashes, and checks all4
container identities, resource budgets, exits and removal. Root independently
confirmed the same28 matching case records. The selector was corrected in c48c927;
no predictions were rerun to produce the derivative receipt.

Raw artifacts remain at
`/vol/md1/tika-indra-artifacts/http-reference-304ccf4-20261018`; its original full
inventory hash is recorded in `acceptance.json`. The separate earlier697 attempt
failed on a misspelled server flag before loading models and was cleaned up;
its directory and original exit143/cleanup0 receipt remain preserved.

The executed gate used one ordinary worker, queue2, batch64, spinning enabled,
threads equal to the CPU cap,3GiB memory and zero swap. Affinity12/12,13 was used
for correctness only. CPU/memory snapshots are execution diagnostics. These
responses do not establish an850-tensor replay of this server, representative
quality, telemetry SDK behavior, signed-release admission or performance.
