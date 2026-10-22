//! Shell-free orchestration for the scratch native acceptance image.
use clap::{Parser, Subcommand};
use miette::Diagnostic;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
};
use thiserror::Error;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Mode,
}
#[derive(Subcommand)]
enum Mode {
    /// Explicitly acquire and verify the immutable text and math artifacts.
    Models {
        #[arg(long, default_value = "/models")]
        directory: PathBuf,
    },
    /// Run actual static test executables; no compilation happens here.
    Test {
        #[arg(long, default_value = "/models")]
        models: PathBuf,
        #[arg(long, default_value = "/evidence")]
        evidence: PathBuf,
        #[arg(long, default_value_t = 1)]
        threads: usize,
    },
}
#[derive(Debug, Error, Diagnostic)]
enum Error {
    #[error("native acceptance I/O failed")]
    Io(#[from] std::io::Error),
    #[error("pinned model acquisition failed")]
    Models(#[from] indra_models::CacheError),
    #[error("native acceptance gate failed: {0}")]
    Gate(&'static str),
}

fn gate(name: &'static str, command: &mut Command, evidence: &Path) -> Result<(), Error> {
    let out = File::create(evidence.join(format!("{name}.stdout.log")))?;
    let err = File::create(evidence.join(format!("{name}.stderr.log")))?;
    let status = command.stdout(out).stderr(err).status()?;
    fs::write(
        evidence.join(format!("{name}.status.txt")),
        status.to_string(),
    )?;
    if !status.success() {
        return Err(Error::Gate(name));
    }
    Ok(())
}

fn run(mode: Mode) -> Result<(), Error> {
    match mode {
        Mode::Models { directory } => {
            let report = indra_models::fetch(&directory, true)?;
            println!(
                "{{\"files\":{},\"bytes\":{},\"downloaded\":{},\"reused\":{}}}",
                report.files, report.bytes, report.downloaded, report.reused
            );
        }
        Mode::Test {
            models,
            evidence,
            threads,
        } => {
            if ![1, 2].contains(&threads) {
                return Err(Error::Gate("threads must be one or two"));
            }
            fs::create_dir_all(&evidence)?;
            for (source, name) in [
                ("/sys/fs/cgroup/cpu.max", "cpu.max"),
                (
                    "/sys/fs/cgroup/cpuset.cpus.effective",
                    "cpuset.cpus.effective",
                ),
                ("/sys/fs/cgroup/memory.max", "memory.max"),
                ("/proc/cpuinfo", "cpuinfo.txt"),
            ] {
                fs::write(evidence.join(name), fs::read(source)?)?;
            }
            let quota = fs::read_to_string("/sys/fs/cgroup/cpu.max")?;
            let mut fields = quota.split_whitespace();
            let limit = fields.next().and_then(|value| value.parse::<usize>().ok());
            let period = fields.next().and_then(|value| value.parse::<usize>().ok());
            if fields.next().is_some()
                || period.is_none_or(|value| value == 0)
                || period.and_then(|value| value.checked_mul(threads)) != limit
            {
                return Err(Error::Gate(
                    "actual CPU quota differs from the declared gate",
                ));
            }
            let thread_text = threads.to_string();
            gate(
                "unit",
                Command::new("/gates/unit")
                    .env("INDRA_TEST_MODEL_DIR", &models)
                    .env("INDRA_TEST_THREADS", &thread_text)
                    .env("RUST_MIN_STACK", "8388608")
                    .args(["--include-ignored", "--test-threads=1"]),
                &evidence,
            )?;
            gate(
                "native",
                Command::new("/gates/native")
                    .env("INDRA_TEST_MODEL_DIR", &models)
                    .env("INDRA_TEST_THREADS", &thread_text)
                    .env("RUST_MIN_STACK", "8388608")
                    .args(["--ignored", "--test-threads=1"]),
                &evidence,
            )?;
            // Zero-churn control operations, the fixed-array selection source and
            // the warmed request path all run on the exact static test binaries.
            gate(
                "allocations",
                &mut Command::new("/gates/allocations"),
                &evidence,
            )?;
            gate(
                "continuous-plan",
                Command::new("/gates/continuous-plan").args(["--test-threads=1"]),
                &evidence,
            )?;
            gate(
                "request-allocations",
                Command::new("/gates/request-allocations")
                    .env("INDRA_TEST_MODEL_DIR", &models)
                    .env("INDRA_TEST_THREADS", &thread_text),
                &evidence,
            )?;
            // Bit-exact n-gram lookups and beam searches against fixtures that
            // the unmodified indra-ocr 1.1.2 produced on the locked archives.
            gate(
                "lm-parity",
                Command::new("/gates/lm-parity")
                    .env("INDRA_TEST_MODEL_DIR", &models)
                    .env("RUST_MIN_STACK", "8388608")
                    .args(["--ignored", "--test-threads=1"]),
                &evidence,
            )?;
            for (name, image) in [
                ("smoke", "/fixtures/smoke.png"),
                ("tiff", "/fixtures/two-pages.tiff"),
            ] {
                gate(
                    name,
                    Command::new("/ocr")
                        .arg(image)
                        .arg("--model-dir")
                        .arg(&models)
                        .args(["--threads", &thread_text, "--cv-threads", &thread_text])
                        .arg("--output")
                        .arg(evidence.join(format!("{name}.txt")))
                        .arg("--stats")
                        .arg(evidence.join(format!("{name}.jsonl"))),
                    &evidence,
                )?;
            }
            println!("native gates passed with threads={threads}");
        }
    }
    Ok(())
}
fn main() -> miette::Result<()> {
    run(Args::parse().command).map_err(miette::Report::new)
}
