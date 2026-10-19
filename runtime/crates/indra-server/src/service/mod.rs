//! One I/O runtime, fixed FIFO storage and dedicated persistent native workers.

mod http;
mod state;
#[cfg(test)]
mod tests;
mod worker;

use crate::{configuration::Settings, error::StartupError};
use indra_core::lifecycle::Phase;
use state::Shared;
use std::{
    future::IntoFuture,
    net::SocketAddr,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub fn api_document() -> utoipa::openapi::OpenApi {
    http::document()
}

pub async fn serve(settings: Settings) -> Result<(), StartupError> {
    settings.validate()?;
    let listener = tokio::net::TcpListener::bind(settings.server.listen).await?;
    let shared = Shared::initialize(settings)?;
    let startup_deadline =
        Instant::now() + Duration::from_secs(shared.settings.startup_timeout_seconds);
    let mut workers = worker::start(shared, startup_deadline)?;
    tracing::info!(event = "listener_bound", listen = %shared.settings.listen, workers = shared.settings.workers,
        admission_capacity = shared.settings.admission_capacity(), "HTTP probes available during model warmup");
    let server = axum::serve(
        listener,
        http::router(shared).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        while !shared.stop_http.load(Ordering::Acquire) {
            shared.http_stop.notified().await;
        }
    })
    .into_future();
    tokio::pin!(server);
    let (result, server_finished) = tokio::select! {
        result = supervise(shared, startup_deadline, &workers) => (result, false),
        result = &mut server => {
            shared.fail("HTTP listener stopped");
            (match result { Ok(()) => Err(StartupError::Terminal("HTTP listener stopped")), Err(error) => Err(StartupError::Io(error)) }, true)
        }
    };
    shared.stop_workers.store(true, Ordering::Release);
    shared.worker_wake.notify_all();
    shared.stop_http.store(true, Ordering::Release);
    shared.http_stop.notify_one();
    if !server_finished {
        match tokio::time::timeout(Duration::from_secs(2), &mut server).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) if result.is_ok() => return Err(StartupError::Io(error)),
            Err(_) if result.is_ok() => return Err(StartupError::DrainDeadline),
            _ => {}
        }
    }
    // Join only handles proven finished. A stuck native call has a finite
    // watchdog outcome; dropping its handle never frees its borrowed buffers.
    for slot in &mut workers {
        if slot
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            let handle = slot.take().expect("finished worker exists");
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) if result.is_ok() => return Err(error),
                Err(_) if result.is_ok() => return Err(StartupError::Worker),
                _ => {}
            }
        }
    }
    result
}

async fn supervise(
    shared: &'static Shared,
    startup_deadline: Instant,
    workers: &worker::Workers,
) -> Result<(), StartupError> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut interval = tokio::time::interval(Duration::from_millis(25));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut draining_at = None;
    loop {
        let now = Instant::now();
        let phase = shared.lifecycle.snapshot().phase;
        if phase == Phase::Starting {
            if shared.ready_workers.load(Ordering::Acquire) == shared.settings.workers {
                shared
                    .lifecycle
                    .activate()
                    .map_err(|_| StartupError::Terminal("activation race"))?;
                shared.warmed.store(true, Ordering::Release);
                tracing::info!(
                    event = "service_ready",
                    model_revision = indra_models::MODEL_REVISION,
                    "All required workers verified and warmed"
                );
            } else if now >= startup_deadline {
                shared.fail("startup deadline");
                return Err(StartupError::StartupDeadline);
            }
        }
        for (index, running) in shared.native_deadlines.iter().enumerate() {
            let deadline = *running
                .lock()
                .map_err(|_| StartupError::Terminal("native status poison"))?;
            if deadline.is_some_and(|deadline| {
                now >= deadline + Duration::from_secs(shared.settings.native_timeout_grace_seconds)
            }) {
                shared.fail("native deadline watchdog");
                indra_telemetry::report_failure(&indra_telemetry::FailureContext {
                    class: "native_deadline_watchdog",
                    phase: indra_telemetry::FailurePhase::Watchdog,
                    request_id: None,
                    worker: Some(index as u8),
                    elapsed: Duration::from_secs(
                        shared.settings.request_timeout_seconds
                            + shared.settings.native_timeout_grace_seconds,
                    ),
                });
                return Err(StartupError::Terminal("native deadline watchdog"));
            }
        }
        if !shared.stop_workers.load(Ordering::Acquire)
            && workers
                .iter()
                .flatten()
                .any(std::thread::JoinHandle::is_finished)
        {
            shared.fail("native worker exited");
        }
        let snapshot = shared.lifecycle.snapshot();
        if matches!(snapshot.phase, Phase::Draining | Phase::Failed) {
            let begin = *draining_at.get_or_insert(now);
            if snapshot.active == 0 {
                shared.stop_workers.store(true, Ordering::Release);
                shared.worker_wake.notify_all();
                if workers
                    .iter()
                    .flatten()
                    .all(std::thread::JoinHandle::is_finished)
                {
                    if let Some(reason) = shared.terminal_reason() {
                        return Err(StartupError::Terminal(reason));
                    }
                    shared
                        .lifecycle
                        .stop()
                        .map_err(|_| StartupError::Terminal("stop ownership"))?;
                    tracing::info!(
                        event = "drain_complete",
                        elapsed_ms = begin.elapsed().as_secs_f64() * 1000.0,
                        "Accepted work and response buffers released"
                    );
                    return Ok(());
                }
            }
            if now.duration_since(begin)
                >= Duration::from_secs(shared.settings.shutdown_timeout_seconds)
            {
                shared.fail("shutdown deadline");
                return Err(StartupError::DrainDeadline);
            }
        }
        tokio::select! {
            _ = interval.tick() => {},
            _ = shared.changed.notified() => {},
            signal = &mut interrupt, if draining_at.is_none() => { signal?; shared.drain("sigint"); },
            signal = terminate.recv(), if draining_at.is_none() => {
                if signal.is_none() { return Err(StartupError::Terminal("SIGTERM stream closed")); }
                shared.drain("sigterm");
            },
        }
    }
}
