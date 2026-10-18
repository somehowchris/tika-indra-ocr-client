# Completed descriptive native profile, baseline 2193324

Normal and optimized independent admission both passed after the controller
finished. Source 2193324, original protocol 6cce0a00968a3532de728b6751039c27e1046a061050c2856bc2e0b5b73a678a,
14 locked model files and tool image 1d14bce8 were verified. Each CPU cap contains
all 46 declared Criterion cases with 100 finite samples per case. Actual affinity
is CPU0 and CPU0,CPU1. All eight phase exits/resource captures, PID ownership,
no-OOM and private container/volume cleanup checks passed. The host bundle stayed
0700 UID/GID 1000; its copied type/mode inventory matches.

| Capture | Samples | Lost | Samples with unknown frames |
| --- | ---: | ---: | ---: |
| 1 CPU TIFF |278|0|0|
| 1 CPU math |761|0|2|
| 2 CPU TIFF |1213|0|0|
| 2 CPU math |2174|0|3|

All recorded process IDs and folded/raw weights match. The four original PNGs
were independently viewed after capture; the separate visual-review receipt
records their unchanged hashes. The program's earlier visual-pending text and
original sample-quality receipts remain untouched. Rust v0 labels are still
mangled. Broad ONNX worker-loop stacks appear in the two-CPU views; interpret
these with retained raw stacks and a separate spin experiment. Kernel names do
not prove ISA speedups.

A semantic code search ran approximately 22:35–22:36 UTC during two-CPU Criterion,
as reported by the operator. This is an approximate disclosure, not an exact
activity trace. Existing host workloads were present. Retain operator window
process/load evidence separately. These are descriptive warmed synthetic
fixture/control-path observations, with no idle-host, causal speedup, Tika
throughput, OCR quality, population-latency or cold-start claim.

Full raw perf streams, original binary/source/relink bundle and command logs
remain at /vol/md1/tika-indra-artifacts/profile-baseline-private-v2-2193324-20261018;
normal/-O command transcripts remain at the sibling profile-admission-private-v2
folder named in their command receipts. This compact packet includes original
figures/folded weights, per-request observations, Criterion samples and phase
receipts. Prior failed preflights and the interrupted first private profile are
preserved; this successful run does not replace them.


The eight raw perf report/script streams are included as deterministic `gzip -n`
files with raw and compressed hashes. Replay the original independent validator
without the external perf.data files:

```sh
bash scripts/replay-compact-native-profile.sh \
  tests/evidence/2026-10-18-native-profile-baseline \
  /absolute/frozen-2193324-source \
  2193324bf42984c9dd3009fd4226b96f2379fad3 \
  /absolute/new-replay-output
```

The four original root-owned 0600 perf.data files remain untouched externally;
`raw-perf-identity/identities.json` binds their SHA256 and lengths. A bounded
capability-free, read-only, network-free tool container with matching owner UID0
read only four explicit file mounts. Before/after modes, sizes and owners matched;
exact owned cleanup and absence passed. The earlier 65534 owner assumption was
rejected before container creation and is retained. It arose from the sandbox's
unprivileged metadata ownership mapping; actual host ownership is 0/0.
