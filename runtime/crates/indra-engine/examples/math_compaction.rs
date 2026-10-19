//! Explicit synthetic math cohort experiment. The service does not expose this feature.
use clap::{Parser, ValueEnum};
use indra_engine::{Engine, EngineConfig, EngineError, MAX_OUTPUT_BYTES, MathArm};
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
    /// Wait up to 30 seconds for the host to admit this exact owned cgroup.
    #[arg(long)]
    host_admission: Option<PathBuf>,
    /// Write an actual ordinary Engine::recognize trace for the embedded pinned fixture.
    #[arg(long)]
    record_reference: bool,
    #[arg(long, requires = "record_reference")]
    output: Option<PathBuf>,
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
    #[error("math experiment failed ({experiment}); resource capture also failed ({evidence})")]
    IncompleteEvidence {
        experiment: EngineError,
        evidence: io::Error,
    },
}
fn run(args: Args) -> Result<(), Error> {
    await_host(args.host_admission.as_deref())?;
    fs::create_dir(&args.resources)?;
    resources(&args.resources, "before")?;
    let result = run_inner(&args);
    let captured = resources(&args.resources, "after");
    match (result, captured) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(experiment), Ok(())) => Err(Error::Engine(experiment)),
        (Ok(()), Err(evidence)) => Err(Error::Io(evidence)),
        (Err(experiment), Err(evidence)) => Err(Error::IncompleteEvidence {
            experiment,
            evidence,
        }),
    }
}
fn run_inner(args: &Args) -> Result<(), EngineError> {
    let config = EngineConfig {
        threads: args.threads,
        cv_threads: args.threads,
        math: true,
        ..EngineConfig::default()
    };
    if args.record_reference
        && (args.output.is_none() || args.gate || args.warmup || args.iterations != 1)
    {
        return Err(EngineError::Config(
            "reference capture requires output, one iteration, no gate and no warmup",
        ));
    }
    if args.iterations == 0 || args.gate && (args.iterations != 1 || args.warmup) {
        return Err(EngineError::Config(
            "gate requires one iteration and no warmup; timing requires at least one iteration",
        ));
    }
    let arm = match args.arm {
        Arm::Ordinary => MathArm::Ordinary,
        Arm::Compact => MathArm::Compact,
    };
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
        ));
    }
    let mut engine = Engine::load(&args.model_dir, &config)?;
    if args.record_reference {
        engine.set_trace_directory(&args.reference)?;
        let mut output = String::with_capacity(MAX_OUTPUT_BYTES);
        engine.recognize(
            include_bytes!("../tests/data/math.png"),
            Instant::now() + Duration::from_secs(300),
            &mut output,
        )?;
        let bytes = output.len();
        fs::write(
            args.output
                .as_ref()
                .ok_or(EngineError::Config("reference output path"))?,
            output,
        )?;
        println!("{{\"reference_created\":true,\"text_bytes\":{bytes}}}");
        return Ok(());
    }
    if args.warmup {
        engine.warmup(Instant::now() + Duration::from_secs(300))?;
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
    Ok(())
}
fn await_host(path: Option<&Path>) -> Result<(), EngineError> {
    let Some(path) = path else {
        return Ok(());
    };
    let due = Instant::now() + Duration::from_secs(30);
    loop {
        match fs::read_to_string(path) {
            Ok(marker) if marker == "admitted\n" => return Ok(()),
            Ok(_) => return Err(EngineError::Config("invalid host cgroup admission marker")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= due {
            return Err(EngineError::Deadline);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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
        fs::write(
            directory.join(format!("{name}.{phase}")),
            fs::read_to_string(source)?,
        )?;
    }
    Ok(())
}
fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}
