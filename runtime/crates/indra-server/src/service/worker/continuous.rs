//! The optional driver owns exactly two existing admissions. Refills occur only
//! between synchronous engine operations; no asynchronous task owns native data.

use std::{sync::atomic::Ordering, time::Instant};

use indra_engine::{ContinuousRequest, Engine, EngineError};

use super::{ActiveCall, Shared, StartupError, Work, finish, report};
use crate::error::ErrorCode;

struct Running {
    work: Work,
    sequence: u64,
    began: Instant,
}

fn take(work: Work, sequence: &mut u64) -> Result<Running, StartupError> {
    *sequence = sequence
        .checked_add(1)
        .ok_or(StartupError::Terminal("continuous sequence overflow"))?;
    indra_telemetry::queue_wait(work.queued_at.elapsed());
    Ok(Running {
        work,
        sequence: *sequence,
        began: Instant::now(),
    })
}

pub(super) fn run(
    shared: &'static Shared,
    worker: u8,
    engine: Engine,
    startup_deadline: Instant,
) -> Result<(), StartupError> {
    // The first page state moves from the warmed normal engine; only the second
    // owner gets additional fixed startup storage. Sessions are not duplicated.
    let mut engine = engine.into_continuous().map_err(|error| {
        report(worker, None, std::time::Duration::ZERO, &error);
        StartupError::Worker
    })?;
    if Instant::now() >= startup_deadline {
        return Err(StartupError::StartupDeadline);
    }
    shared.ready_workers.fetch_add(1, Ordering::AcqRel);
    shared.changed.notify_one();
    let mut owners: [Option<Running>; 2] = [const { None }; 2];
    let mut sequence = 0_u64;
    loop {
        if let Some(reason) = shared.terminal_reason() {
            // A watchdog can fail the service during a native call. This
            // boundary runs only after that call returned, so it is safe to
            // release retained work and forbidden to start another operation.
            fail_owners(&mut owners);
            return Err(StartupError::Terminal(reason));
        }
        if owners.iter().all(Option::is_none) {
            let Some(work) = shared.next_work()? else {
                return Ok(());
            };
            owners[0] = Some(take(work, &mut sequence)?);
        }
        for owner in &mut owners {
            if owner.is_none()
                && let Some(work) = shared.try_work()?
            {
                *owner = Some(take(work, &mut sequence)?);
            }
        }
        let span = tracing::info_span!(
            "ocr.continuous_operation",
            worker,
            owners = owners.iter().filter(|owner| owner.is_some()).count()
        );
        for owner in owners.iter().flatten() {
            // A shared native invocation has both admitted request contexts as
            // causal links. Request IDs/document contents are not metric labels.
            span.follows_from(owner.work.span.id());
        }
        let _entered = span.enter();
        let active = ActiveCall::idle(shared, worker);
        let [first, second] = &mut owners;
        let requests = [first, second].map(|owner| {
            owner.as_mut().map(|owner| {
                let buffers = owner.work.reservation.buffers_mut();
                ContinuousRequest {
                    sequence: owner.sequence,
                    input: &buffers.input,
                    output: &mut buffers.text,
                    deadline: owner.work.deadline,
                    cancelled: owner.work.response.is_closed(),
                }
            })
        });
        let result = engine.advance(requests, |deadline| {
            if shared.terminal_reason().is_some() {
                return Err(EngineError::Config("continuous service is terminal"));
            }
            active
                .set(deadline)
                .map_err(|_| EngineError::Config("continuous watchdog status poison"))
        });
        // No reservation can be dropped before the shared operation returned.
        drop(active);
        let progress = match result {
            Ok(progress) => progress,
            Err(error) => {
                super::report_phase(
                    worker,
                    None,
                    std::time::Duration::ZERO,
                    &error,
                    indra_telemetry::FailurePhase::Inference,
                );
                shared.fail(super::class(&error));
                fail_owners(&mut owners);
                return Err(StartupError::Worker);
            }
        };
        if let Some(invocation) = progress.invocation {
            tracing::info!(
                event = "continuous_invocation",
                worker,
                model = invocation.model,
                shared = invocation.shared(),
                owner_zero_rows = invocation.owner_rows[0],
                owner_one_rows = invocation.owner_rows[1],
                padded_width = invocation.padded_width,
                elapsed_ms = invocation.elapsed.as_secs_f64() * 1000.0,
                "Actual stateless model invocation completed"
            );
        }
        for (slot, completion) in owners.iter_mut().zip(progress.completed) {
            if let Some(completion) = completion {
                let owner = slot
                    .take()
                    .ok_or(StartupError::Terminal("continuous completion ownership"))?;
                finish(shared, worker, owner.work, owner.began, completion)?;
            }
        }
    }
}

fn fail_owners(owners: &mut [Option<Running>; 2]) {
    for owner in owners {
        if let Some(owner) = owner.take() {
            let _ = owner.work.response.send(Err(ErrorCode::NativeFailure));
        }
    }
}
