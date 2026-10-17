//! Admission and drain use one atomic word, so a successful admission cannot race
//! past the transition to draining. A lease follows accepted work into its worker.

use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

#[derive(Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum Phase {
    Starting = 0,
    Ready = 1,
    Draining = 2,
    Stopped = 3,
    Failed = 4,
}

impl Phase {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    const fn from_word(word: u64) -> Self {
        match word & 7 {
            0 => Self::Starting,
            1 => Self::Ready,
            2 => Self::Draining,
            3 => Self::Stopped,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AdmissionError {
    #[error("OCR service is not accepting work ({0:?})")]
    Unavailable(Phase),
    #[error("OCR admission capacity is occupied")]
    Saturated,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TransitionError {
    #[error("service transition is invalid from {0:?}")]
    Phase(Phase),
    #[error("accepted OCR work is still active")]
    ActiveWork,
}

#[derive(Debug)]
pub struct Snapshot {
    pub phase: Phase,
    pub active: u16,
}

impl Snapshot {
    const fn from_word(word: u64) -> Self {
        Self {
            phase: Phase::from_word(word),
            active: (word >> 3) as u16,
        }
    }
}

#[derive(Debug)]
pub struct Lifecycle {
    word: AtomicU64,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Lifecycle {
    pub const fn new() -> Self {
        Self {
            word: AtomicU64::new(Phase::Starting as u64),
        }
    }

    /// Call only after every required model and worker is ready.
    pub fn activate(&self) -> Result<(), TransitionError> {
        self.word
            .compare_exchange(
                Phase::Starting as u64,
                Phase::Ready as u64,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|word| TransitionError::Phase(Phase::from_word(word)))
    }

    /// One successful CAS is the admission linearization point. The capacity
    /// includes uploading, queued and executing requests, not just queue entries.
    pub fn admit(&self, capacity: u16) -> Result<Lease<'_>, AdmissionError> {
        let mut current = self.word.load(Ordering::Acquire);
        loop {
            let state = Snapshot::from_word(current);
            if state.phase != Phase::Ready {
                return Err(AdmissionError::Unavailable(state.phase));
            }
            if state.active >= capacity {
                return Err(AdmissionError::Saturated);
            }
            match self.word.compare_exchange_weak(
                current,
                current + 8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Lease { lifecycle: self }),
                Err(observed) => current = observed,
            }
        }
    }

    /// Immediately removes readiness. Existing leases remain valid. Repeated
    /// drain requests cannot reopen admission or overwrite a terminal failure.
    pub fn drain(&self) -> Snapshot {
        let mut current = self.word.load(Ordering::Acquire);
        loop {
            let state = Snapshot::from_word(current);
            match state.phase {
                Phase::Starting | Phase::Ready => {}
                _ => return state,
            }
            let desired = (current & !7) | Phase::Draining as u64;
            match self.word.compare_exchange_weak(
                current,
                desired,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Snapshot::from_word(desired),
                Err(observed) => current = observed,
            }
        }
    }

    pub fn fail(&self) {
        self.word.fetch_or(4, Ordering::AcqRel);
    }

    /// Stopping is legal only after the final lease has been released.
    pub fn stop(&self) -> Result<(), TransitionError> {
        self.word
            .compare_exchange(
                Phase::Draining as u64,
                Phase::Stopped as u64,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|word| {
                let state = Snapshot::from_word(word);
                if state.active > 0 {
                    TransitionError::ActiveWork
                } else {
                    TransitionError::Phase(state.phase)
                }
            })
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::from_word(self.word.load(Ordering::Acquire))
    }
}

/// A lease is deliberately non-clonable. Moving it into a worker makes client
/// disconnection unable to release capacity while inference still runs.
#[derive(Debug)]
pub struct Lease<'a> {
    lifecycle: &'a Lifecycle,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let previous = self.lifecycle.word.fetch_sub(8, Ordering::AcqRel);
        debug_assert!(previous >> 3 > 0, "lease accounting underflow");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    #[test]
    fn startup_and_terminal_states_never_admit() {
        let lifecycle = Lifecycle::new();
        assert!(matches!(
            lifecycle.admit(1),
            Err(AdmissionError::Unavailable(Phase::Starting))
        ));
        assert_eq!(lifecycle.drain().phase, Phase::Draining);
        assert!(lifecycle.activate().is_err());
        lifecycle.stop().expect("drained startup can stop");
        assert!(lifecycle.admit(1).is_err());
        assert!(lifecycle.activate().is_err());
    }

    #[test]
    fn accepted_work_survives_drain_until_its_lease_is_dropped() {
        let lifecycle = Lifecycle::new();
        lifecycle.activate().expect("starting");
        let lease = lifecycle.admit(1).expect("available slot");
        assert!(matches!(lifecycle.admit(1), Err(AdmissionError::Saturated)));
        let drained = lifecycle.drain();
        assert_eq!(drained.phase, Phase::Draining);
        assert_eq!(drained.active, 1);
        assert_eq!(lifecycle.stop(), Err(TransitionError::ActiveWork));
        assert!(matches!(
            lifecycle.admit(1),
            Err(AdmissionError::Unavailable(Phase::Draining))
        ));
        drop(lease);
        lifecycle.stop().expect("all work completed");
        assert_eq!(lifecycle.snapshot().active, 0);
    }

    #[test]
    fn failure_is_terminal_and_preserves_outstanding_work() {
        let lifecycle = Lifecycle::new();
        lifecycle.activate().expect("starting");
        let lease = lifecycle.admit(1).expect("available slot");
        lifecycle.fail();
        assert_eq!(lifecycle.snapshot().phase, Phase::Failed);
        assert_eq!(lifecycle.drain().phase, Phase::Failed);
        assert!(lifecycle.admit(1).is_err());
        drop(lease);
        assert_eq!(lifecycle.snapshot().active, 0);
        assert!(lifecycle.stop().is_err());
    }

    #[test]
    fn concurrent_admission_cannot_escape_the_capacity_or_reopen_drain() {
        for _ in 0..128 {
            let lifecycle = Lifecycle::new();
            lifecycle.activate().expect("starting");
            let start = Barrier::new(3);
            thread::scope(|scope| {
                let first = scope.spawn(|| {
                    start.wait();
                    lifecycle.admit(1).ok()
                });
                let second = scope.spawn(|| {
                    start.wait();
                    lifecycle.admit(1).ok()
                });
                start.wait();
                lifecycle.drain();
                let a = first.join().expect("first admission thread");
                let b = second.join().expect("second admission thread");
                assert!(a.is_none() || b.is_none());
                assert!(lifecycle.admit(1).is_err());
                drop(a);
                drop(b);
            });
            lifecycle.stop().expect("drain is complete");
        }
    }

    #[test]
    fn cancelled_receiver_does_not_release_worker_owned_capacity() {
        let lifecycle = Lifecycle::new();
        lifecycle.activate().expect("starting");
        let lease = lifecycle.admit(1).expect("available slot");
        let (response, receiver) = std::sync::mpsc::sync_channel::<u8>(1);
        let worker_started = Barrier::new(2);
        let worker_finished = Barrier::new(2);
        thread::scope(|scope| {
            scope.spawn(|| {
                let work_lease = lease;
                worker_started.wait();
                worker_finished.wait();
                assert!(response.send(42).is_err());
                drop(work_lease);
            });
            worker_started.wait();
            drop(receiver);
            assert_eq!(lifecycle.snapshot().active, 1);
            assert!(matches!(lifecycle.admit(1), Err(AdmissionError::Saturated)));
            lifecycle.drain();
            assert_eq!(lifecycle.stop(), Err(TransitionError::ActiveWork));
            worker_finished.wait();
        });
        lifecycle
            .stop()
            .expect("worker completed after cancellation");
    }

    #[test]
    fn maximum_capacity_does_not_wrap_the_count_into_phase_bits() {
        let lifecycle = Lifecycle::new();
        lifecycle.activate().expect("starting");
        let mut leases: [Option<Lease<'_>>; u16::MAX as usize] =
            [const { None }; u16::MAX as usize];
        for slot in &mut leases {
            *slot = Some(lifecycle.admit(u16::MAX).expect("remaining capacity"));
        }
        assert_eq!(lifecycle.snapshot().active, u16::MAX);
        assert!(matches!(
            lifecycle.admit(u16::MAX),
            Err(AdmissionError::Saturated)
        ));
        lifecycle.drain();
        for slot in &mut leases {
            drop(slot.take());
        }
        lifecycle.stop().expect("every lease was released");
    }
}
