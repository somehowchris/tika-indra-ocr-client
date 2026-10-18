# Same-age math compaction: measured synthetic study

All forty timed arms, two fresh ordinary production captures and four numerical
gates completed with `exit=0`, `cleanup_exit=0`, no OOM and every owned container
removed. The unchanged numerical gates passed complete accepted UTF-8, discrete
row/stop decisions and mapped tensors at finite `atol=rtol=1e-5`. Compaction remains
an opt-in diagnostic. These results do not establish OCR quality or page/Tika
performance.

Ten alternating ordinary/compact pairs ran at each CPU budget. Ratios below are
compact divided by ordinary, calculated per pair; values below one are lower.

| CPU budget | Cohort ratio min / median / max | Decoder ratio min / median / max | Slower compact cohorts |
| --- | --- | --- | ---: |
| 1 | 0.7506 / 0.7805 / 0.7901 | 0.5905 / 0.6193 / 0.6270 | 0/10 |
| 2 | 0.6064 / 0.9294 / 1.2753 | 0.3506 / 0.8613 / 1.3566 | 3/10 |

The two-CPU variation includes a 27.5% cohort regression. Neither the median nor
fewer decoder row-steps supports enabling this experiment in production. This
one synthetic six-row cohort provides descriptive observations, without a
population confidence interval or an explanation of the variation.

`cohort_ns` includes encoder/cache/state preparation, decoder calls, actual
copies, input metadata, accepted-text validation and final cache release.
`decoder_ns` starts after preparation. Neither includes model/session loading,
reference file reads or page preprocessing. Every timed arm validated complete
production accepted UTF-8; expensive numerical comparisons and immediate gather
bit verification were disabled only for timing, as specified before prediction.

The fields named `full_arm_*` cover the recorded resource snapshot interval.
`cpu.stat.before` is read after CLI setup and host admission; `cpu.stat.after` is
read after model loading/verification, all-session and selected-arm warmup,
reference preparation, the cohort, reporting and engine teardown, before the
remaining final snapshots and process exit. These are not literal whole-process
measurements. Memory peak is the cgroup high-water mark when read. Median paired
CPU ratios for this interval were 0.9478/0.9810 at one/two CPUs; median memory
peak ratios were 0.9985/1.0014. They cannot be assigned to the timed decoder alone.
Zero-denominator throttling ratios remain `null` in the report.

The run used AMD Ryzen 9 7950X3D physical cores 0 and 0,1, admitted as distinct
cores on the same socket/L3. Fresh containers had exact one/two-CPU CFS quotas,
3 GiB memory, zero swap, 128 PIDs, no network, read-only root/model mounts,
UID 65532 and no capabilities. Each live host PID, Docker leaf, CPU affinity and
CPU/memory ancestors were checked at a bounded startup barrier. Local compilation,
other inference and profiling had stopped before this controlled window.

`cell-identities.jsonl` records the independent metadata admission of all 46
roles. `admit-portable-metadata.sh` retains the exact checks against original
container, host PID/cgroup, actual limits/affinity, command, exit, OOM and removal
records. `cell-admission.json` is its successful verdict. The original raw records
remain in the artifact directory below. `paired.json` and all twenty `pair-*.json`
retain each observation and paired difference/ratio. Only resource-scope wording
in this portable `paired.json` was clarified after the run; all numeric data,
original raw analysis and frozen protocol are unchanged.

The four Criterion cases at each cap measure one hot self-cache K or V tensor,
with the same arena reused without reset between iterations. They exclude the
second self cache, cross caches, mask, tokens, input views, ORT, page preprocessing
and Tika. Each case used a one-second warmup, three-second measurement and 100
samples. The four cases move 357,120 bytes in total for one cache; this is not
an end-to-end latency model.

| Single self-cache gather | 1 CPU mean, ns | 2 CPU mean, ns |
| --- | ---: | ---: |
| 6 to 4 at step 8 | 886.85 | 890.00 |
| 4 to 3 at step 23 | 1,401.68 | 1,400.27 |
| 3 to 2 at step 27 | 1,422.44 | 1,442.76 |
| 2 to 1 at step 32 | 966.55 | 933.77 |

Eight `criterion-cpu*.json` files retain benchmark identity, all 100 raw samples
and Criterion estimates. Its confidence intervals describe each microbenchmark;
they do not quantify cross-profile or production uncertainty. Actual controller
and inner script, selected Cargo artifact, resource/cleanup verdict and source
hashes are retained.

The producing source was `dfef64b97b26d729e69d835dd71ba045bd166b97`; the timed
controller was `50c6b65`. The native diagnostic was the immutable scratch image
`sha256:7e196e2d42a9ca54b5ee00789e17c8205b941a83e6effdccc1f79201c744b7e3`,
with executed static ELF SHA-256
`c1d638ab980ef3bd60f2f0d8f63b9ff6297c462eb5026f727709b0df164da4b9`.
The caller-locked receipt SHA-256 is
`3cf89f068f55db1f38d173f80c4cbddeea71902e53ec3d746a6b13fb64da6b42`.
Its exact protocol, Cargo lockfile, native archive/glibc hashes, verified model
lock/manifest and source archive hash are retained here. This was a local static
relink from pinned recovered archives, not a fresh native-library build or signed
release. Full source/native/model bytes remain bound by the receipt.

The Criterion executable was relinked from that exact source, using the same
verified native archive set. Cargo completed successfully in 1m48s; the following
export step failed with 127 because its builder lacked `jq`. That attempt is
retained. A separate bounded export-only attempt selected the unique successful
Cargo JSON bench artifact, checked its path/type/freshness/source, and exported it
without compiling. The executed static Criterion ELF SHA-256 is
`3392fc381c72885db2dcc303b44321468edcb3b20a8182c9fcef9c55a15b0e6a`.
It ran read-only in tools image
`sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142`
with the same admitted caps and an 8 MiB stack. Export-only and measurement
cleanup both succeeded. No stale always-verifying Criterion binary was measured.

Original local evidence, including failed attempts and complete raw resources:

- `/vol/md1/tika-indra-artifacts/math-compaction-paired-dfef64b-20261018`
- `/vol/md1/tika-indra-artifacts/math-gather-criterion-dfef64b-20261018`
- `/vol/md1/tika-indra-artifacts/math-compaction-bench-dfef64b-20261018`
- `/vol/md1/tika-indra-artifacts/math-compaction-bench-export-dfef64b-20261018`

This portable packet omits large model/tensor/archive/ELF data. Existing
source/native/model/binary hash manifests bind those original materials; no
large artifacts were rescanned during the later ablation window.

`criterion-resources-cpu1.json` and `criterion-resources-cpu2.json` also retain
original container inspect objects, raw before/after cgroup/process snapshots,
host ancestor checks and exit/removal records. Binary hashes, ELF inspection and
successful export status are separate from the failed original export-stage
status, preserving that distinction.
