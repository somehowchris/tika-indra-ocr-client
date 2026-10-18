//! Explicit synthetic math cohort experiment. The service does not expose this feature.
use clap::{Parser, ValueEnum};
use indra_engine::{Engine, EngineConfig, EngineError, MathArm};
use miette::Diagnostic;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use thiserror::Error;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(ValueEnum, Clone, Copy)]
enum Arm {
    Ordinary,
    Compact,
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    model_dir: PathBuf,
    /// New directory for actual cgroup/process snapshots; must not already exist.
    #[arg(long)]
    resources: PathBuf,
    /// Trace from ordinary Engine::recognize, containing exactly one six-row math cohort.
    #[arg(long)]
    reference: PathBuf,
    #[arg(long, value_enum, default_value_t = Arm::Compact)]
    arm: Arm,
    #[arg(long, default_value_t = 1)]
    threads: usize,
    /// Check production snapshots and the independent mapped ordinary stream; timing is invalid.
    #[arg(long)]
    gate: bool,
    /// One unmeasured selected-arm pass before measurement. Models are always verified first.
    #[arg(long)]
    warmup: bool,
    #[arg(long, default_value_t = 1)]
    iterations: usize,
}
#[derive(Debug, Error, Diagnostic)]
enum Error {
    #[error("math compaction experiment failed")]
    Engine(#[from] EngineError),
    #[error("math experiment resource evidence failed")]
    Io(#[from] io::Error),
}
fn run(args: Args) -> Result<(), Error> {
    let config = EngineConfig {
        threads: args.threads,
        cv_threads: args.threads,
        math: true,
        ..EngineConfig::default()
    };
    if args.iterations == 0 || args.gate && (args.iterations != 1 || args.warmup) {
        return Err(EngineError::Config(
            "gate requires one iteration and no warmup; timing requires at least one iteration",
        )
        .into());
    }
    let arm = match args.arm {
        Arm::Ordinary => MathArm::Ordinary,
        Arm::Compact => MathArm::Compact,
    };
    fs::create_dir(&args.resources)?;
    resources(&args.resources, "before")?;
    let quota = fs::read_to_string("/sys/fs/cgroup/cpu.max")?;
    let mut values = quota.split_whitespace();
    let limit = values.next().and_then(|value| value.parse::<usize>().ok());
    let period = values.next().and_then(|value| value.parse::<usize>().ok());
    if values.next().is_some()
        || period.is_none_or(|period| period == 0)
        || period.and_then(|period| period.checked_mul(args.threads)) != limit
        || fs::read_to_string("/sys/fs/cgroup/memory.max")?.trim() != "3221225472"
        || fs::read_to_string("/sys/fs/cgroup/memory.swap.max")?.trim() != "0"
    {
        return Err(EngineError::Config(
            "actual experiment CPU/memory/swap cap differs from protocol",
        )
        .into());
    }
    let mut engine = Engine::load(&args.model_dir, &config)?;
    if args.warmup {
        engine.math_compaction_experiment(
            &args.reference,
            arm,
            false,
            Instant::now() + Duration::from_secs(300),
        )?;
    }
    for iteration in 0..args.iterations {
        let report = engine.math_compaction_experiment(
            &args.reference,
            arm,
            args.gate,
            Instant::now() + Duration::from_secs(300),
        )?;
        println!(
            "{{\"schema_version\":1,\"arm\":\"{}\",\"gate\":{},\"timing_valid\":{},\"threads\":{},\"iteration\":{},\"cohort_ns\":{},\"decoder_ns\":{},\"row_steps\":{},\"gathered_bytes_cross_self_mask_tokens\":{:?},\"verified_gather_elements\":{},\"tensor_views\":{},\"live_sizes\":{:?},\"stop_steps\":{:?},\"compared_float_elements\":{}}}",
            match arm {
                MathArm::Ordinary => "ordinary",
                MathArm::Compact => "compact",
            },
            args.gate,
            !args.gate,
            args.threads,
            iteration,
            report.cohort_ns,
            report.decoder_ns,
            report.row_steps,
            report.gathered_bytes,
            report.verified_gather_elements,
            report.tensor_views,
            &report.live_sizes[..report.live_size_count],
            &report.stop_steps[..report.original_rows],
            report.compared_float_elements
        );
    }
    resources(&args.resources, "after")?;
    Ok(())
}
fn resources(directory: &Path, phase: &str) -> io::Result<()> {
    for (source, name) in [
        ("/sys/fs/cgroup/cpu.max", "cpu.max"),
        ("/sys/fs/cgroup/cpu.stat", "cpu.stat"),
        (
            "/sys/fs/cgroup/cpuset.cpus.effective",
            "cpuset.cpus.effective",
        ),
        ("/sys/fs/cgroup/memory.max", "memory.max"),
        ("/sys/fs/cgroup/memory.swap.max", "memory.swap.max"),
        ("/sys/fs/cgroup/memory.peak", "memory.peak"),
        ("/sys/fs/cgroup/memory.events", "memory.events"),
        ("/sys/fs/cgroup/pids.max", "pids.max"),
        ("/proc/self/status", "process.status"),
        ("/proc/cpuinfo", "cpuinfo"),
    ] {
        fs::copy(source, directory.join(format!("{name}.{phase}")))?;
    }
    Ok(())
}
fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}
