//! A small native acceptance and measurement command; the HTTP worker uses the same Engine.

use std::{
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::Parser;
use indra_engine::{
    Engine, EngineConfig, EngineError, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, OcrStats,
};
use miette::Diagnostic;
use opencv::{
    core::{self, Mat, Scalar},
    prelude::*,
};
use thiserror::Error;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(about = "Run pinned native Indra OCR and emit real per-request stage/batch measurements")]
struct Args {
    input: PathBuf,
    #[arg(long)]
    model_dir: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
    #[arg(long, default_value_t = 1)]
    cv_threads: usize,
    #[arg(long, default_value_t = 64)]
    batch_size: usize,
    #[arg(long)]
    math: bool,
    #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
    intra_op_spinning: bool,
    #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
    inter_op_spinning: bool,
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,
    #[arg(long, default_value_t = 1)]
    iterations: usize,
    #[arg(long)]
    warmup: bool,
    #[arg(long)]
    output: Option<PathBuf>,
    /// JSON Lines measurements; stderr is used when this is omitted.
    #[arg(long)]
    stats: Option<PathBuf>,
    /// An empty directory for diagnostic tensors; cannot be combined with repeated timing.
    #[arg(long)]
    trace_directory: Option<PathBuf>,
    /// Compare final UTF-8 output byte-for-byte with a reference file.
    #[arg(long)]
    expect: Option<PathBuf>,
}

#[derive(Debug, Error, Diagnostic)]
enum CliError {
    #[error("native OCR failed")]
    Engine(#[from] EngineError),
    #[error("input/output operation failed")]
    Io(#[from] io::Error),
    #[error("invalid command: {0}")]
    Arguments(&'static str),
    #[error("native UTF-8 output differs from the supplied reference")]
    Parity,
}

fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}

fn run(args: Args) -> Result<(), CliError> {
    if args.timeout_seconds == 0
        || args.timeout_seconds > 3600
        || !(1..=1000).contains(&args.iterations)
    {
        return Err(CliError::Arguments(
            "timeout must be 1..=3600 seconds and iterations 1..=1000",
        ));
    }
    if args.trace_directory.is_some() && args.iterations != 1 {
        return Err(CliError::Arguments(
            "tensor evidence uses one run; repeat measurements with tensor export disabled",
        ));
    }
    let settings = EngineConfig {
        threads: args.threads,
        cv_threads: args.cv_threads,
        intra_op_spinning: args.intra_op_spinning,
        inter_op_spinning: args.inter_op_spinning,
        math: args.math,
        batch_size: args.batch_size,
    };
    let mut file = File::open(&args.input)?;
    let length = usize::try_from(file.metadata()?.len())
        .map_err(|_| CliError::Arguments("input is too large"))?;
    if length == 0 || length > MAX_INPUT_BYTES {
        return Err(CliError::Arguments(
            "encoded input must be 1 byte to 20 MiB",
        ));
    }
    let mut bytes =
        Mat::new_rows_cols_with_default(1, length as i32, core::CV_8UC1, Scalar::all(0.0))
            .map_err(EngineError::from)?;
    file.read_exact(bytes.data_typed_mut::<u8>().map_err(EngineError::from)?)?;
    if file.read(&mut [0_u8; 1])? != 0 {
        return Err(CliError::Arguments("input changed during reading"));
    }
    let mut engine = Engine::load(&args.model_dir, &settings)?;
    let timeout = Duration::from_secs(args.timeout_seconds);
    if args.warmup {
        engine.warmup(Instant::now() + timeout)?;
    }
    if let Some(path) = &args.trace_directory {
        engine.set_trace_directory(path)?;
    }
    let mut stats_file = args.stats.map(File::create).transpose()?;
    let mut output = String::with_capacity(MAX_OUTPUT_BYTES);
    for iteration in 0..args.iterations {
        let start = Instant::now();
        let stats = engine.recognize(
            bytes.data_typed::<u8>().map_err(EngineError::from)?,
            start + timeout,
            &mut output,
        )?;
        let elapsed = start.elapsed();
        if let Some(file) = &mut stats_file {
            write_stats(
                file,
                iteration,
                elapsed,
                &stats,
                args.trace_directory.is_some(),
            )?;
        } else {
            write_stats(
                &mut io::stderr().lock(),
                iteration,
                elapsed,
                &stats,
                args.trace_directory.is_some(),
            )?;
        }
    }
    if let Some(expected) = args.expect {
        let expected = std::fs::read_to_string(expected)?;
        if expected != output {
            return Err(CliError::Parity);
        }
    }
    if let Some(path) = args.output {
        std::fs::write(path, output)?;
    } else {
        io::stdout().lock().write_all(output.as_bytes())?;
    }
    Ok(())
}

fn write_stats(
    writer: &mut impl Write,
    iteration: usize,
    elapsed: Duration,
    stats: &OcrStats,
    diagnostic: bool,
) -> io::Result<()> {
    write!(
        writer,
        "{{\"iteration\":{iteration},\"elapsed_us\":{},\"diagnostic\":{diagnostic},\"threads\":{},\"cv_threads\":{},\"intra_op_spinning\":{},\"inter_op_spinning\":{},\"math\":{},\"batch_size\":{},\"frames\":{},\"lines\":{},\"output_bytes\":{},\"valid_columns\":{},\"padded_columns\":{},\"batch_counts\":[",
        elapsed.as_micros(),
        stats.settings.threads,
        stats.opencv_threads,
        stats.settings.intra_op_spinning,
        stats.settings.inter_op_spinning,
        stats.settings.math,
        stats.settings.batch_size,
        stats.frames,
        stats.lines,
        stats.output_bytes,
        stats.valid_columns,
        stats.padded_columns
    )?;
    for (index, count) in stats.batch_counts.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        write!(writer, "{count}")?;
    }
    writer.write_all(b"],\"stage_us\":[")?;
    for (index, time) in stats.stage_times.iter().enumerate() {
        if index > 0 {
            writer.write_all(b",")?;
        }
        write!(writer, "{}", time.as_micros())?;
    }
    writer.write_all(b"]}\n")
}
