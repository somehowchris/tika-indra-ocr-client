use super::state::{Completed, Shared, Work};
use crate::{
    configuration::MAX_WORKERS,
    error::{ErrorCode, StartupError},
};
use indra_engine::{Engine, EngineError};
use std::{
    sync::atomic::Ordering,
    thread::JoinHandle,
    time::{Duration, Instant},
};

mod continuous;

pub(super) type Workers = [Option<JoinHandle<Result<(), StartupError>>>; MAX_WORKERS];

// The fixed engine state and its native initialization exceed Rust's default
// 2 MiB worker stack. Give initialization and inference bounded headroom;
// reserve it once per persistent worker, within the service memory limit.
static NATIVE_STACK_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn start(
    shared: &'static Shared,
    startup_deadline: Instant,
) -> Result<Workers, StartupError> {
    let mut workers = [const { None }; MAX_WORKERS];
    for (index, slot) in workers
        .iter_mut()
        .enumerate()
        .take(usize::from(shared.settings.workers))
    {
        let worker = index as u8;
        *slot = Some(
            std::thread::Builder::new()
                .name(format!("indra-native-{worker}"))
                .stack_size(NATIVE_STACK_BYTES)
                .spawn(move || {
                    let result = run(shared, worker, startup_deadline);
                    if result.is_err() {
                        shared.fail("native worker stopped");
                    }
                    result
                })
                .map_err(|_| StartupError::Worker)?,
        );
    }
    Ok(workers)
}

fn run(shared: &'static Shared, worker: u8, startup_deadline: Instant) -> Result<(), StartupError> {
    let startup = tracing::info_span!(
        "ocr.worker.startup",
        worker,
        stack_bytes = NATIVE_STACK_BYTES,
        model_revision = indra_models::MODEL_REVISION,
        ort_threads = shared.engine.threads,
        cv_threads = shared.engine.cv_threads,
        batch_size = shared.engine.batch_size,
        math = shared.engine.math,
        experimental_continuous_batching = shared.settings.experimental_continuous_batching
    );
    let mut engine = startup.in_scope(|| {
        let begin = Instant::now();
        let mut engine = Engine::load(&shared.model_dir, &shared.engine).map_err(|error| {
            report(shared, worker, None, begin.elapsed(), &error);
            StartupError::Worker
        })?;
        engine.warmup(startup_deadline).map_err(|error| {
            report(shared, worker, None, begin.elapsed(), &error);
            StartupError::Worker
        })?;
        tracing::info!(
            event = "worker_warmed",
            elapsed_ms = begin.elapsed().as_secs_f64() * 1000.0,
            "Models verified and worker warmed"
        );
        Ok::<_, StartupError>(engine)
    })?;
    if shared.settings.experimental_continuous_batching {
        return continuous::run(shared, worker, engine, startup_deadline);
    }
    shared.ready_workers.fetch_add(1, Ordering::AcqRel);
    shared.changed.notify_one();
    while let Some(work) = shared.next_work()? {
        process(shared, worker, &mut engine, work)?;
    }
    Ok(())
}

fn process(
    shared: &'static Shared,
    worker: u8,
    engine: &mut Engine,
    work: Work,
) -> Result<(), StartupError> {
    let Work {
        mut reservation,
        response,
        request_id,
        deadline,
        queued_at,
        span,
    } = work;
    let _entered = span.enter();
    indra_telemetry::queue_wait(queued_at.elapsed());
    if response.is_closed() || Instant::now() >= deadline {
        let _ = response.send(Err(ErrorCode::Deadline));
        tracing::info!(
            event = "queued_work_skipped",
            request_id = request_id.as_str(),
            worker,
            "Cancelled or expired request skipped before native work"
        );
        return Ok(());
    }
    let active = ActiveCall::start(shared, worker, deadline)?;
    let begin = Instant::now();
    let buffers = reservation.buffers_mut();
    let result = engine.recognize(&buffers.input, deadline, &mut buffers.text);
    drop(active);
    drop(_entered);
    finish(
        shared,
        worker,
        Work {
            reservation,
            response,
            request_id,
            deadline,
            queued_at,
            span,
        },
        begin,
        result,
    )
}

/// Completion owns the reservation until the HTTP body's final byte reference
/// drops. Both execution modes use the same response and terminal-fault policy.
fn finish(
    shared: &'static Shared,
    worker: u8,
    work: Work,
    begin: Instant,
    result: indra_engine::Result<indra_engine::OcrStats>,
) -> Result<(), StartupError> {
    let Work {
        reservation,
        response,
        request_id,
        deadline,
        span,
        ..
    } = work;
    let _entered = span.enter();
    match result {
        Ok(stats) => {
            indra_telemetry::inference(&indra_telemetry::InferenceMetrics {
                stages: &stats.stage_times,
                batches: &stats.batch_counts,
                valid_columns: stats.valid_columns,
                padded_columns: stats.padded_columns,
                frames: stats.frames,
                lines: stats.lines,
                input_bytes: reservation.buffers().input.len(),
                output_bytes: stats.output_bytes,
            });
            tracing::info!(
                event = "inference_finished",
                request_id = request_id.as_str(),
                worker,
                elapsed_ms = begin.elapsed().as_secs_f64() * 1000.0,
                frames = stats.frames,
                lines = stats.lines,
                output_bytes = stats.output_bytes,
                valid_columns = stats.valid_columns,
                padded_columns = stats.padded_columns,
                "Native OCR completed"
            );
            if Instant::now() >= deadline {
                let _ = response.send(Err(ErrorCode::Deadline));
            } else if stats.output_bytes > shared.settings.max_response_bytes {
                let _ = response.send(Err(ErrorCode::TooLarge));
            } else {
                // On send failure the completion owns and releases the buffers;
                // no HTTP-task cancellation can bypass native ownership.
                let _ = response.send(Ok(Completed { reservation, stats }));
            }
            Ok(())
        }
        Err(error) => {
            let code = classify(&error);
            if matches!(code, ErrorCode::NativeFailure) {
                report(
                    shared,
                    worker,
                    Some(request_id.as_str()),
                    begin.elapsed(),
                    &error,
                );
                shared.fail_with(
                    class(&error),
                    indra_telemetry::FailurePhase::Inference,
                    Some(worker),
                );
            } else {
                tracing::warn!(
                    event = "inference_rejected",
                    request_id = request_id.as_str(),
                    worker,
                    error_code = code.name(),
                    "OCR request could not be processed"
                );
            }
            let _ = response.send(Err(code));
            if matches!(code, ErrorCode::NativeFailure) {
                Err(StartupError::Worker)
            } else {
                Ok(())
            }
        }
    }
}

fn classify(error: &EngineError) -> ErrorCode {
    match error {
        EngineError::InvalidImage(_) | EngineError::UnsupportedImage(_) => ErrorCode::InvalidImage,
        EngineError::Limit(_) => ErrorCode::TooLarge,
        EngineError::Deadline => ErrorCode::Deadline,
        _ => ErrorCode::NativeFailure,
    }
}

fn class(error: &EngineError) -> &'static str {
    match error {
        EngineError::InvalidImage(_) => "invalid_image",
        EngineError::UnsupportedImage(_) => "unsupported_image",
        EngineError::Limit(_) => "resource_limit",
        EngineError::Deadline => "deadline",
        EngineError::Config(_) => "engine_configuration",
        EngineError::Shape(_) => "model_shape",
        EngineError::Cache(_) => "model_cache",
        EngineError::Ort(_) => "onnx_runtime",
        EngineError::OpenCv(_) => "opencv",
        EngineError::Io(_) => "engine_io",
    }
}

fn report(
    shared: &'static Shared,
    worker: u8,
    request_id: Option<&str>,
    elapsed: Duration,
    error: &EngineError,
) {
    report_phase(
        shared,
        worker,
        request_id,
        elapsed,
        error,
        if request_id.is_some() {
            indra_telemetry::FailurePhase::Inference
        } else {
            indra_telemetry::FailurePhase::Startup
        },
    );
}

fn report_phase(
    shared: &'static Shared,
    worker: u8,
    request_id: Option<&str>,
    elapsed: Duration,
    error: &EngineError,
    phase: indra_telemetry::FailurePhase,
) {
    shared.report_once(&indra_telemetry::FailureContext {
        class: class(error),
        phase,
        request_id,
        worker: Some(worker),
        elapsed,
    });
}

struct ActiveCall {
    shared: &'static Shared,
    worker: u8,
}

impl ActiveCall {
    const fn idle(shared: &'static Shared, worker: u8) -> Self {
        Self { shared, worker }
    }

    fn set(&self, deadline: Instant) -> Result<(), StartupError> {
        *self.shared.native_deadlines[usize::from(self.worker)]
            .lock()
            .map_err(|_| StartupError::Terminal("native status poison"))? = Some(deadline);
        Ok(())
    }

    fn start(shared: &'static Shared, worker: u8, deadline: Instant) -> Result<Self, StartupError> {
        let active = Self::idle(shared, worker);
        active.set(deadline)?;
        Ok(active)
    }
}

impl Drop for ActiveCall {
    fn drop(&mut self) {
        if let Ok(mut deadline) = self.shared.native_deadlines[usize::from(self.worker)].lock() {
            *deadline = None;
        } else {
            self.shared.fail("native status poison");
        }
        self.shared.changed.notify_one();
    }
}
