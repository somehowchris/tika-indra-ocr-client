//! Prospective native-only scheduler experiment. Never a quality benchmark.
//! Twelve fixed admissions; exactly one native worker and at most two owners.
use clap::{Parser, ValueEnum};
use indra_engine::{
    ContinuousRequest, Engine, EngineConfig, EngineError, MAX_OUTPUT_BYTES, OcrStats,
};
use miette::Diagnostic;
use sha2::{Digest, Sha256};
use stats_alloc::{Region, StatsAlloc};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;

#[path = "continuous_experiment/schedule.rs"]
mod schedule;
use schedule::{INPUTS, REQUESTS, Schedule};

// The dependency's historically named feature only exposes this const generic
// constructor; it compiles on the pinned stable compiler. No custom unsafe code.
#[global_allocator]
static ALLOCATOR: StatsAlloc<mimalloc::MiMalloc> = StatsAlloc::new(mimalloc::MiMalloc);
static FIXTURES: [&[u8]; 3] = [
    include_bytes!("../../../../backend/tests/data/smoke.png"),
    include_bytes!("../../../../backend/tests/data/two-pages.tiff"),
    include_bytes!("../tests/data/math.png"),
];
static NAMES: [&str; 3] = ["smoke", "two-pages", "math-page"];

#[derive(Clone, Copy, ValueEnum)]
enum Arm {
    Ordinary,
    Continuous,
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    model_dir: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_enum)]
    arm: Arm,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=3))]
    concurrency: u8,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    cpus: u8,
    #[arg(long, default_value_t=300, value_parser=clap::value_parser!(u16).range(1..=3600))]
    timeout_seconds: u16,
    /// Optional separate cancellation probe, never mixed into paired timings.
    #[arg(long)]
    cancel_first: bool,
}
#[derive(Debug, Error, Diagnostic)]
enum Failure {
    #[error("experiment I/O failed")]
    Io(#[from] io::Error),
    #[error("native inference failed")]
    Engine(#[from] EngineError),
    #[error("experiment admission failed: {0}")]
    Gate(&'static str),
}
struct Outcome {
    elapsed_us: u128,
    queue_us: u128,
    output_sha256: [u8; 32],
    stats: Option<OcrStats>,
    status: &'static str,
}
impl Outcome {
    const fn pending() -> Self {
        Self {
            elapsed_us: 0,
            queue_us: 0,
            output_sha256: [0; 32],
            stats: None,
            status: "missing",
        }
    }
}
fn main() -> miette::Result<()> {
    run(Args::parse()).map_err(miette::Report::new)
}
fn run(args: Args) -> Result<(), Failure> {
    fs::create_dir(&args.output)?;
    let mut journal = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(args.output.join("requests.jsonl"))?;
    let settings = EngineConfig {
        threads: usize::from(args.cpus),
        cv_threads: usize::from(args.cpus),
        intra_op_spinning: true,
        inter_op_spinning: true,
        math: false,
        batch_size: 64,
    };
    let timeout = Duration::from_secs(u64::from(args.timeout_seconds));
    snapshot(&args.output, "startup", args.cpus)?;
    let mut engine = Engine::load(&args.model_dir, &settings)?;
    engine.warmup(Instant::now() + timeout)?;
    let mut reference: [String; 3] =
        std::array::from_fn(|_| String::with_capacity(MAX_OUTPUT_BYTES));
    for (index, output) in reference.iter_mut().enumerate() {
        engine.recognize(FIXTURES[index], Instant::now() + timeout, output)?;
        fs::write(
            args.output.join(format!("reference-{}.txt", NAMES[index])),
            output.as_bytes(),
        )?;
    }
    let mut outputs: [String; 2] = std::array::from_fn(|_| String::with_capacity(MAX_OUTPUT_BYTES));
    let mut outcomes: [Outcome; REQUESTS] = std::array::from_fn(|_| Outcome::pending());
    let mut admissions = [None; REQUESTS];
    let mut began = [None; REQUESTS];
    let mut schedule = Schedule::new(usize::from(args.concurrency));
    let mut shared_invocations = 0_u64;
    let mut invocation_rows = [0_u64; 64];
    let mut total_model_rows = 0_u64;
    let mut model_padded_columns = 0_u64;
    let mut owners: [Option<usize>; 2] = [None, None];
    // Allocate the second page state and every output buffer before measurement.
    // Both variants stay on the protocol's bounded 8 MiB native stack; no per-request heap indirection.
    #[allow(clippy::large_enum_variant)]
    enum Worker {
        Ordinary(Engine),
        Continuous(indra_engine::Continuous),
    }
    let mut worker = match args.arm {
        Arm::Ordinary => Worker::Ordinary(engine),
        Arm::Continuous => Worker::Continuous(engine.into_continuous()?),
    };
    snapshot(&args.output, "measurement-before", args.cpus)?;
    let allocation_region = Region::new(&ALLOCATOR);
    let start = Instant::now();
    let mut terminal = false;
    let mut terminal_error = "none";
    while !schedule.complete() && !terminal {
        while let Some(index) = schedule.admit() {
            admissions[index] = Some(Instant::now());
        }
        match &mut worker {
            Worker::Ordinary(engine) => {
                let Some(index) = schedule.take() else {
                    return Err(Failure::Gate("ordinary queue empty"));
                };
                let now = Instant::now();
                began[index] = Some(now);
                let result = if args.cancel_first && index == 0 {
                    Err(EngineError::Deadline)
                } else {
                    engine.recognize(
                        FIXTURES[INPUTS[index]],
                        admissions[index].unwrap_or(now) + timeout,
                        &mut outputs[0],
                    )
                };
                record(
                    index,
                    result,
                    &outputs[0],
                    &reference,
                    &admissions,
                    &began,
                    &mut outcomes,
                );
                schedule.finish(index).map_err(Failure::Gate)?;
            }
            Worker::Continuous(engine) => {
                for owner in &mut owners {
                    if owner.is_none() {
                        *owner = schedule.take();
                        if let Some(index) = *owner {
                            began[index] = Some(Instant::now());
                        }
                    }
                }
                let progress = {
                    let [first, second] = &mut outputs;
                    let requests = owners.iter().zip([first, second]).map(|(owner, output)| {
                        owner.map(|index| ContinuousRequest {
                            sequence: index as u64 + 1,
                            input: FIXTURES[INPUTS[index]],
                            output,
                            deadline: admissions[index].unwrap_or(start) + timeout,
                            cancelled: args.cancel_first && index == 0,
                        })
                    });
                    let mut requests = requests;
                    engine.advance(
                        [requests.next().flatten(), requests.next().flatten()],
                        |_| Ok(()),
                    )
                };
                match progress {
                    Ok(progress) => {
                        if let Some(invocation) = progress.invocation {
                            let rows = invocation.owner_rows.iter().sum::<usize>();
                            if !(1..=64).contains(&rows) {
                                return Err(Failure::Gate("invocation rows exceed bound"));
                            }
                            shared_invocations += u64::from(invocation.shared());
                            invocation_rows[rows - 1] += 1;
                            total_model_rows += rows as u64;
                            model_padded_columns += rows as u64 * invocation.padded_width as u64;
                        }
                        for (slot, result) in progress.completed.into_iter().enumerate() {
                            if let Some(result) = result {
                                let index = owners[slot]
                                    .take()
                                    .ok_or(Failure::Gate("completion without owner"))?;
                                record(
                                    index,
                                    result,
                                    &outputs[slot],
                                    &reference,
                                    &admissions,
                                    &began,
                                    &mut outcomes,
                                );
                                schedule.finish(index).map_err(Failure::Gate)?;
                            }
                        }
                    }
                    Err(error) => {
                        terminal = true;
                        terminal_error = error_code(&error);
                    }
                }
            }
        }
    }
    let wall = start.elapsed();
    let allocations = allocation_region.change();
    snapshot(&args.output, "measurement-after", args.cpus)?;
    let mut successes = 0;
    let mut successful_latencies = [0_u128; REQUESTS];
    for (index, outcome) in outcomes.iter().enumerate() {
        if outcome.status == "success" {
            successful_latencies[successes] = outcome.elapsed_us;
            successes += 1;
        }
        write!(
            journal,
            "{{\"sequence\":{index},\"fixture\":\"{}\",\"status\":\"{}\"",
            NAMES[INPUTS[index]], outcome.status
        )?;
        if outcome.status == "missing" {
            write!(
                journal,
                ",\"elapsed_us\":null,\"queue_us\":null,\"output_sha256\":null"
            )?;
        } else {
            write!(
                journal,
                ",\"elapsed_us\":{},\"queue_us\":{},\"output_sha256\":\"",
                outcome.elapsed_us, outcome.queue_us
            )?;
            for byte in outcome.output_sha256 {
                write!(journal, "{byte:02x}")?;
            }
            write!(journal, "\"")?;
        }
        if let Some(stats) = &outcome.stats {
            write!(
                journal,
                ",\"frames\":{},\"lines\":{},\"valid_columns\":{},\"padded_columns\":{},\"output_bytes\":{},\"recognition_batch_counts\":[",
                stats.frames,
                stats.lines,
                stats.valid_columns,
                stats.padded_columns,
                stats.output_bytes
            )?;
            for (bin, count) in stats.batch_counts.iter().enumerate() {
                if bin > 0 {
                    write!(journal, ",")?;
                }
                write!(journal, "{count}")?;
            }
            write!(journal, "]")?;
        } else {
            write!(
                journal,
                ",\"frames\":null,\"lines\":null,\"valid_columns\":null,\"padded_columns\":null,\"output_bytes\":null,\"recognition_batch_counts\":null"
            )?;
        }
        writeln!(journal, "}}")?;
    }
    journal.sync_all()?;
    successful_latencies[..successes].sort_unstable();
    let mut summary = File::create(args.output.join("summary.json"))?;
    write!(
        summary,
        "{{\"schema_version\":1,\"scope\":\"native-only scheduler; not HTTP, quality, or continuous-math evidence\",\"arm\":\"{}\",\"cpus\":{},\"concurrency\":{},\"planned_requests\":12,\"successful_requests\":{successes},\"complete\":{},\"cancellation_probe\":{},\"wall_us\":{},\"successful_responses_per_wall_second\":{},\"shared_invocations\":{shared_invocations},\"invocation_diagnostics_scope\":\"continuous-only; ordinary recognition batches are in each request\",\"actual_model_rows\":",
        match args.arm {
            Arm::Ordinary => "ordinary",
            Arm::Continuous => "continuous",
        },
        args.cpus,
        args.concurrency,
        schedule.complete() && !terminal,
        args.cancel_first,
        wall.as_micros(),
        successes as f64 / wall.as_secs_f64()
    )?;
    match args.arm {
        Arm::Ordinary => write!(
            summary,
            "null,\"invocation_padded_columns\":null,\"model_row_histogram\":null"
        )?,
        Arm::Continuous => write!(
            summary,
            "{total_model_rows},\"invocation_padded_columns\":{model_padded_columns},\"model_row_histogram\":["
        )?,
    }
    if matches!(args.arm, Arm::Continuous) {
        for (index, count) in invocation_rows.iter().enumerate() {
            if index > 0 {
                write!(summary, ",")?;
            }
            write!(summary, "{count}")?;
        }
        write!(summary, "]")?;
    }
    writeln!(
        summary,
        ",\"rust_allocator\":{{\"name\":\"instrumented mimalloc\",\"scope\":\"Rust global allocator only; native ONNX/OpenCV allocations excluded; warmed scheduling and hashing included; JSON serialization excluded\",\"allocations\":{},\"deallocations\":{},\"reallocations\":{},\"bytes_allocated\":{},\"bytes_deallocated\":{},\"bytes_reallocated\":{}}},\"copy_accounting\":{{\"encoded_input_copies_in_scheduler\":0,\"output_reference_comparison_bytes\":{},\"native_internal_copy_bytes\":null,\"native_copy_scope\":\"not instrumented; never reported as zero\"}}}}",
        allocations.allocations,
        allocations.deallocations,
        allocations.reallocations,
        allocations.bytes_allocated,
        allocations.bytes_deallocated,
        allocations.bytes_reallocated,
        outcomes
            .iter()
            .filter_map(|o| o.stats.as_ref())
            .map(|s| s.output_bytes)
            .sum::<usize>()
    )?;
    summary.sync_all()?;
    let mut latency = File::create(args.output.join("latency.json"))?;
    write!(
        latency,
        "{{\"successful_samples\":{successes},\"failed_or_missing_samples\":{},\"method\":\"nearest rank; 12 nested requests cannot resolve a stable p99\",\"p50_us\":",
        REQUESTS - successes
    )?;
    for (index, percent) in [50, 95, 99].into_iter().enumerate() {
        if index == 1 {
            write!(latency, ",\"p95_us\":")?;
        } else if index == 2 {
            write!(latency, ",\"p99_us\":")?;
        }
        if successes == 0 {
            write!(latency, "null")?;
        } else {
            write!(
                latency,
                "{}",
                successful_latencies[(successes * percent).div_ceil(100) - 1]
            )?;
        }
    }
    writeln!(
        latency,
        ",\"terminal_error_category\":\"{terminal_error}\"}}"
    )?;
    latency.sync_all()?;
    let cancellation_ok = !args.cancel_first
        || (successes == REQUESTS - 1
            && outcomes[0].status == "deadline_or_cancelled"
            && outcomes[1..].iter().all(|o| o.status == "success"));
    if terminal
        || !schedule.complete()
        || !cancellation_ok
        || (!args.cancel_first && successes != REQUESTS)
    {
        return Err(Failure::Gate(
            "failed outcomes retained; experiment rejected",
        ));
    }
    Ok(())
}
fn record(
    index: usize,
    result: Result<OcrStats, EngineError>,
    output: &str,
    reference: &[String; 3],
    admissions: &[Option<Instant>; REQUESTS],
    began: &[Option<Instant>; REQUESTS],
    outcomes: &mut [Outcome; REQUESTS],
) {
    let now = Instant::now();
    let arrived = admissions[index].unwrap_or(now);
    let status = match &result {
        Ok(_) => {
            if output == reference[INPUTS[index]] {
                "success"
            } else {
                "output_mismatch"
            }
        }
        Err(EngineError::Deadline) => "deadline_or_cancelled",
        Err(_) => "engine_failure",
    };
    outcomes[index] = Outcome {
        elapsed_us: now.duration_since(arrived).as_micros(),
        queue_us: began[index]
            .unwrap_or(now)
            .duration_since(arrived)
            .as_micros(),
        output_sha256: Sha256::digest(output.as_bytes()).into(),
        stats: result.ok(),
        status,
    };
}

fn snapshot(output: &std::path::Path, label: &str, cpus: u8) -> Result<(), Failure> {
    let group = std::path::Path::new("/sys/fs/cgroup");
    let directory = output.join(label);
    fs::create_dir(&directory)?;
    let quota = fs::read_to_string(group.join("cpu.max"))?;
    let mut fields = quota.split_whitespace();
    let quota = fields
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(Failure::Gate("finite actual CPU quota required"))?;
    let period = fields
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or(Failure::Gate("finite CPU period required"))?;
    if period != 100_000
        || quota != u64::from(cpus) * period
        || fs::read_to_string(group.join("memory.max"))?.trim() != "3221225472"
        || fs::read_to_string(group.join("memory.swap.max"))?.trim() != "0"
    {
        return Err(Failure::Gate(
            "actual cgroup quota differs from frozen experiment",
        ));
    }
    for name in [
        "cpu.max",
        "cpu.stat",
        "cpuset.cpus.effective",
        "memory.max",
        "memory.swap.max",
        "memory.current",
        "memory.peak",
        "memory.events",
        "pids.max",
        "pids.current",
    ] {
        fs::write(directory.join(name), fs::read(group.join(name))?)?;
    }
    fs::write(
        directory.join("process-status"),
        fs::read("/proc/self/status")?,
    )?;
    fs::write(
        directory.join("process-cgroup"),
        fs::read("/proc/self/cgroup")?,
    )?;
    Ok(())
}

const fn error_code(error: &EngineError) -> &'static str {
    match error {
        EngineError::InvalidImage(_) => "invalid_image",
        EngineError::UnsupportedImage(_) => "unsupported_image",
        EngineError::Limit(_) => "resource_limit",
        EngineError::Deadline => "deadline",
        EngineError::Config(_) => "configuration",
        EngineError::Shape(_) => "tensor_shape",
        EngineError::Cache(_) => "model_cache",
        EngineError::Ort(_) => "onnx",
        EngineError::OpenCv(_) => "opencv",
        EngineError::Io(_) => "io",
    }
}
