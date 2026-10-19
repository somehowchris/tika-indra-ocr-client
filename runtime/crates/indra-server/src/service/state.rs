use crate::{
    configuration::{MAX_SLOTS, MAX_WORKERS, ServerSettings, Settings},
    error::{ErrorCode, StartupError},
    protocol::RequestId,
};
use bytes::BytesMut;
use indra_core::{
    lifecycle::{AdmissionError, Lease, Lifecycle},
    queue::Queue,
};
use indra_engine::{EngineConfig, MAX_OUTPUT_BYTES, OcrStats};
use std::{
    path::PathBuf,
    sync::{
        Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Instant,
};
use tokio::sync::{Notify, oneshot};

static SHARED: OnceLock<Shared> = OnceLock::new();

pub(super) struct Shared {
    pub settings: ServerSettings,
    pub engine: EngineConfig,
    pub model_dir: PathBuf,
    pub lifecycle: Lifecycle,
    pub queue: Mutex<Queue<Work, MAX_SLOTS>>,
    pool: Mutex<[Option<Buffers>; MAX_SLOTS]>,
    pub worker_wake: Condvar,
    pub changed: Notify,
    pub ready_workers: AtomicU8,
    pub warmed: AtomicBool,
    pub stop_workers: AtomicBool,
    pub stop_http: AtomicBool,
    pub http_stop: Notify,
    pub native_deadlines: [Mutex<Option<Instant>>; MAX_WORKERS],
    terminal: Mutex<Option<&'static str>>,
}

impl Shared {
    pub fn initialize(settings: Settings) -> Result<&'static Self, StartupError> {
        let shared = Self::new(settings);
        SHARED
            .set(shared)
            .map_err(|_| StartupError::AlreadyInitialized)?;
        SHARED.get().ok_or(StartupError::AlreadyInitialized)
    }

    pub fn new(settings: Settings) -> Self {
        let capacity = usize::from(settings.server.admission_capacity());
        let pool = std::array::from_fn(|index| {
            (index < capacity).then(|| Buffers {
                input: BytesMut::with_capacity(settings.server.max_request_bytes),
                // Engine output is bounded at this limit. A smaller public response
                // limit must not introduce growth in the worker's reusable string.
                text: String::with_capacity(MAX_OUTPUT_BYTES),
            })
        });
        Self {
            settings: settings.server,
            engine: settings.engine.engine_config(),
            model_dir: settings.models.directory,
            lifecycle: Lifecycle::new(),
            queue: Mutex::new(Queue::new()),
            pool: Mutex::new(pool),
            worker_wake: Condvar::new(),
            changed: Notify::new(),
            ready_workers: AtomicU8::new(0),
            warmed: AtomicBool::new(false),
            stop_workers: AtomicBool::new(false),
            stop_http: AtomicBool::new(false),
            http_stop: Notify::new(),
            native_deadlines: [const { Mutex::new(None) }; MAX_WORKERS],
            terminal: Mutex::new(None),
        }
    }

    pub fn reserve(&'static self) -> Result<Reservation, ErrorCode> {
        let lease = self
            .lifecycle
            .admit(self.settings.admission_capacity())
            .map_err(|error| match error {
                AdmissionError::Saturated => ErrorCode::QueueFull,
                AdmissionError::Unavailable(_) => ErrorCode::Unavailable,
            })?;
        let mut pool = self.pool.lock().map_err(|_| {
            self.fail("buffer pool poison");
            ErrorCode::Internal
        })?;
        let buffer = pool.iter_mut().find_map(Option::take).ok_or_else(|| {
            self.fail_without_queue("buffer pool ownership");
            ErrorCode::Internal
        })?;
        indra_telemetry::admission(1);
        Ok(Reservation {
            shared: self,
            buffer: Some(buffer),
            lease: Some(lease),
        })
    }

    pub fn enqueue(&self, work: Work) -> Result<(), ErrorCode> {
        let mut queue = self.queue.lock().map_err(|_| {
            self.fail_without_queue("queue poison");
            ErrorCode::Internal
        })?;
        if self.stop_workers.load(Ordering::Acquire) || self.terminal_reason().is_some() {
            return Err(ErrorCode::Unavailable);
        }
        // Already-admitted uploads remain eligible after normal drain begins.
        // Admission capacity proves the fixed FIFO cannot overflow here.
        if queue.push(work).is_err() {
            drop(queue);
            self.fail("queue accounting");
            return Err(ErrorCode::Internal);
        }
        drop(queue);
        self.worker_wake.notify_one();
        Ok(())
    }

    pub fn next_work(&self) -> Result<Option<Work>, StartupError> {
        let mut queue = self
            .queue
            .lock()
            .map_err(|_| StartupError::Terminal("queue poison"))?;
        loop {
            if let Some(work) = queue.pop() {
                return Ok(Some(work));
            }
            if self.stop_workers.load(Ordering::Acquire) {
                return Ok(None);
            }
            queue = self
                .worker_wake
                .wait(queue)
                .map_err(|_| StartupError::Terminal("queue poison"))?;
        }
    }

    /// Refill an experimental owner only from the existing admitted FIFO. This
    /// never waits, reserves another buffer, or changes the admission capacity.
    pub fn try_work(&self) -> Result<Option<Work>, StartupError> {
        self.queue
            .lock()
            .map(|mut queue| queue.pop())
            .map_err(|_| StartupError::Terminal("queue poison"))
    }

    pub fn drain(&self, source: &'static str) {
        let snapshot = self.lifecycle.drain();
        tracing::info!(
            event = "drain_started",
            source,
            phase = snapshot.phase.as_str(),
            active = snapshot.active,
            "Readiness removed; accepted work retains ownership"
        );
        self.changed.notify_one();
        self.worker_wake.notify_all();
    }

    pub fn fail(&self, reason: &'static str) {
        self.fail_without_queue(reason);
        if let Ok(mut queue) = self.queue.lock() {
            while let Some(work) = queue.pop() {
                // There is no healthy execution guarantee after a terminal
                // native fault. Every queued caller gets an explicit failure.
                let _ = work.response.send(Err(ErrorCode::NativeFailure));
            }
        }
        self.worker_wake.notify_all();
    }

    fn fail_without_queue(&self, reason: &'static str) {
        self.lifecycle.fail();
        if let Ok(mut terminal) = self.terminal.lock() {
            terminal.get_or_insert(reason);
        }
        self.changed.notify_one();
    }

    pub fn terminal_reason(&self) -> Option<&'static str> {
        self.terminal
            .lock()
            .map_or(Some("terminal status poison"), |reason| *reason)
    }
}

pub(super) struct Buffers {
    pub input: BytesMut,
    pub text: String,
}

/// Owns admission and both reusable buffers until native work AND the final
/// response-byte reference are released. A disconnected HTTP task cannot free
/// native capacity while the worker is still accessing those buffers.
pub(super) struct Reservation {
    shared: &'static Shared,
    buffer: Option<Buffers>,
    lease: Option<Lease<'static>>,
}

impl Reservation {
    pub fn buffers_mut(&mut self) -> &mut Buffers {
        self.buffer.as_mut().expect("live reservation owns buffers")
    }
    pub fn buffers(&self) -> &Buffers {
        self.buffer.as_ref().expect("live reservation owns buffers")
    }
}

impl AsRef<[u8]> for Reservation {
    fn as_ref(&self) -> &[u8] {
        self.buffers().text.as_bytes()
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(mut buffer) = self.buffer.take() {
            buffer.input.clear();
            buffer.text.clear();
            if let Ok(mut pool) = self.shared.pool.lock() {
                if let Some(slot) = pool.iter_mut().find(|slot| slot.is_none()) {
                    *slot = Some(buffer);
                } else {
                    self.shared.fail_without_queue("buffer return accounting");
                }
            } else {
                self.shared.fail_without_queue("buffer return poison");
            }
        }
        drop(self.lease.take());
        indra_telemetry::admission(-1);
        self.shared.changed.notify_one();
        self.shared.worker_wake.notify_all();
    }
}

pub(super) struct Work {
    pub reservation: Reservation,
    pub response: oneshot::Sender<Result<Completed, ErrorCode>>,
    pub request_id: RequestId,
    pub deadline: Instant,
    pub queued_at: Instant,
    pub span: tracing::Span,
}

pub(super) struct Completed {
    pub reservation: Reservation,
    pub stats: OcrStats,
}
