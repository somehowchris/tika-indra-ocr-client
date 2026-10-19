The hosted [profile run39173943401](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39173943401)
failed when `cargo install --path profile-tools/inferno` discovered the application's
owning workspace. Inferno is an independent package, so Cargo rejected the nested
checkout before compiling it. This failure is independent of the deployment
`crictl` version-command correction.

The workflow now exports the exact pinned
[Inferno source929be68](https://github.com/jonhoo/inferno/tree/929be68e19a27da062623c6da5255e8b96faebbc)
into a new directory under `RUNNER_TEMP`, rejects any owning Cargo manifest in
its ancestors, and compiles there. Upstream source, manifest, lockfile, version,
features, toolchain and selected binaries remain unchanged. The source archive
and its SHA256 are retained with the profiler's tool receipts.

The [compact proof](inferno-workspace-isolation-2026-10-19/result.json) retains the
actual nested-workspace error101, successful external locked/offline metadata,
actual two-binary installation, binary identities and a passing100-sample Inferno
collapse fixture. Rust1.99.1 compiled the unchanged upstream lock with two jobs
on CPUs12,13. An intermediate location under `/vol/md1` also had an owning
workspace and was rejected; this motivated the explicit ancestor guard. No OCR
inference, hosted workflow rerun, profiling measurements or speedup is established
by this tool-build proof.
