//! Explicit synthetic math cohort experiment. The service does not expose this feature.
use clap::{Parser, ValueEnum};
use indra_engine::{Engine, EngineConfig, EngineError, MathArm};
use miette::Diagnostic;
use std::{
    path::PathBuf,
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
#[error("math compaction experiment failed")]
struct Error(#[from] EngineError);
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
    Ok(())
}
fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}
