# Gateway Sentry request correlation: actual validation

The gateway returns a 16-hex `X-Request-ID`. Its shared Sentry reporter previously accepted only 32-hex IDs, so a real gateway failure lost its HTTP correlation ID. The correction preserves 32-hex IDs for every failure phase and additionally accepts exactly 16 ASCII hex characters for gateway failures.

Producing source identities are recorded in [source-receipts.json](source-receipts.json):

- Fixed production source: `c405064013b4fcd146f8992e90b49d3cef873a8d`.
- Before-fix comparison: `db859dd34094361771194bacc1dfcf6f973edb10`, which adds the same collector regression to `1b42f4aaf8be3d8652ef37d0861eed8c6be9eb7a` while retaining the old production reporter.

Both producing worktrees were clean when built. This packet adds evidence only. [validation-summary.json](validation-summary.json) binds the exact variants, command statuses and ELF hashes. The backend is the existing Rust process fixture; these runs establish gateway ownership and actual SDK transport behavior. Tika/native OCR quality and performance have separate evidence.

## Results and limits

| Gate | Result | Evidence |
|---|---|---|
| Before-fix real Sentry-only exchange | Expected exit 101: envelope `request_id` is `Null`; returned HTTP ID is `0000000000000001` | [stderr](before-fix/process.stderr), [command/status](before-fix/commands.jsonl) |
| Reporter/telemetry unit suite | 4 passed, including 32-hex compatibility, gateway 16-hex acceptance, native 16-hex rejection and invalid length/nonhex/UTF-8 rejection | [output](fixed-checks-attempt3/reporter-unit-all-features.stdout) |
| Gateway unit suite | 11 passed | [output](fixed-checks-attempt3/gateway-unit-all-features.stdout) |
| Full default-feature process harness, 1 CPU | Passed; CPU 12, `cpu.max=100000 100000`, 3 GiB memory, zero swap | [commands](process-default-1cpu-attempt3/commands.jsonl), [output](process-default-1cpu-attempt3/process.stdout), [kernel limits](process-default-1cpu-attempt3/cgroup-before.json) |
| Full default-feature process harness, 2 CPUs | Passed; CPUs 12–13, `cpu.max=200000 100000`, 3 GiB memory, zero swap | [commands](process-default-2cpu-attempt3/commands.jsonl), [output](process-default-2cpu-attempt3/process.stdout), [kernel limits](process-default-2cpu-attempt3/cgroup-before.json) |
| `--no-default-features --features sentry` collector case | Passed with OTLP excluded at compile time; same 2-CPU/3-GiB/zero-swap limits | [Cargo selection](fixed-checks-attempt3/sentry-only-c405064-a3-selection.json), [commands](process-sentry-only-attempt3/commands.jsonl), [kernel limits](process-sentry-only-attempt3/cgroup-before.json) |
| Strict Clippy | Release, all features/all targets, `-D warnings`: passed | [output](fixed-checks-attempt3/clippy.stderr), [exact commands/statuses](fixed-checks-attempt3/commands.jsonl) |
| Workspace formatting | `cargo fmt --all -- --check`: passed | [exact commands/statuses](fixed-checks-attempt3/commands.jsonl) |
| GNU static ELF checks | `x86_64-unknown-linux-gnu`, `-Ctarget-feature=+crt-static`; no interpreter or dynamic `NEEDED` entries | [ELF receipts](fixed-checks-attempt3/elf-receipts.json), retained `readelf` outputs beside them |

The full process harness exercises command/SIGTERM drain, saturation, an upload admitted before drain, retained parsing after disconnect, truncation, header/request deadlines, version pinning, orphan reaping and the real Sentry correlation case. Process runs include the gateway, fixture children, caller and collector within one bounded container. Their cgroup-after files retain memory/pid peaks and zero OOM-kill counts. [Container receipts](containers/) retain the exact create commands and before/after inspection, including network isolation and absence of privileged mode or socket mounts. These are unmeasured correctness runs.

The pinned compiler is Rust 1.99.1 (`58cf8f9c35b355199727627d07ae34a4e2846d67`); [rustc output](fixed-checks-attempt3/rustc.stdout) and [Cargo output](fixed-checks-attempt3/cargo.stdout) are retained. The existing tooling image was `sha256:56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239`. Builds used its cached registry offline and mounted the pinned lint components read-only. No `DOCS_RS` result is used.

## Producing artifacts and freshness

Only `default-c405064-a3-*` and `sentry-only-c405064-a3-*` are accepted fixed artifacts. Their source/feature combinations used separate newly created target directories. Both changed local packages also received an explicit release clean before the default producing build. The selector rejects any affected local artifact with `fresh != false` and requires a fresh reporter artifact before copying the exact Cargo-reported gateway and harness executables.

- Default: [exact artifact selection](fixed-checks-attempt3/default-c405064-a3-selection.json), [fresh local Cargo JSON](fixed-checks-attempt3/default-c405064-a3-fresh-local-artifacts.json).
- Sentry-only: [exact artifact selection](fixed-checks-attempt3/sentry-only-c405064-a3-selection.json), [fresh local Cargo JSON](fixed-checks-attempt3/sentry-only-c405064-a3-fresh-local-artifacts.json).
- Before-fix: [exact artifact selection](before-fix/before-selection.json), [fresh local Cargo JSON](before-fix/before-fresh-local-artifacts.json).

The retained [controllers](controllers/) show each harness invocation's explicit `INDRA_TEST_GATEWAY_BIN` binding to the corresponding selected executable. Large binaries, duplicate target trees and complete dependency compiler streams remain outside this portable packet; affected-package Cargo records, hashes, selected paths, build diagnostics and ELF inspection outputs are included.

## Retained raw fault envelope

A follow-up bounded capture executed the verified Sentry-only gateway ELF and existing fixture ELF directly, without rebuilding. The actual upstream EOF returned HTTP 502 and emitted one bounded SDK envelope. The [HTTP response](fault-envelope-capture/gateway-response.http), [raw envelope](fault-envelope-capture/sentry.envelope), [decoded event](fault-envelope-capture/sentry-event.json) and [result](fault-envelope-capture/result.json) retain matching `0000000000000001` IDs and successful Java/orphan reaping. [The command/environment](fault-envelope-capture/gateway-command.json) contains only fixture settings and a public local test DSN; [kernel limits](fault-envelope-capture/kernel-limits.json) confirm the cap. Canary document/auth/cookie values are absent from the envelope, and request/user/hostname payload fields are absent.

The Sentry SDK supplies an independent propagation context in `contexts.trace` even with OTLP excluded. It does not establish an OTLP parent edge. Correlation in this case is proved by the event's `request_id` matching the returned HTTP ID.

## Failed attempts

All setup failures are explicitly retained and indexed in [failures/index.json](failures/index.json): sandbox Docker denial, unavailable noninteractive sudo, an optional-field inspection template failure, an unsuitable uncached tool image, the initial evidence-directory permission failure, the wrong non-release clean, the initial rustup launcher network-sync failure, and a corrected relative path during packet checksum generation. The initial clean removed zero files and allowed an old Sentry-only artifact copy to survive. Its selections are retained as **invalidated evidence**, with no accepted fixed process claim. The final fresh-directory builds and checked Cargo records above supersede those copies. No source, request protocol or privacy rule was relaxed to obtain a pass.

`SHA256SUMS` covers every file in this packet except itself.
