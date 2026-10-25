use crate::{
    configuration::{Gateway, MAX_ACTIVE, Settings},
    error::{self, Error, ProxyError},
    listener::{LimitedListener, Peer},
    process::Java,
    proxy,
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::BytesMut;
use indra_core::lifecycle::{AdmissionError, Lease, Lifecycle, Phase};
use serde::Serialize;
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, Semaphore},
    time::Instant,
};
use utoipa::{OpenApi, ToSchema};

static SHARED: OnceLock<Shared> = OnceLock::new();

pub(crate) struct Shared {
    pub settings: Gateway,
    pub lifecycle: Lifecycle,
    pub connections: Semaphore,
    pub changed: Notify,
    pub java_stopped: AtomicBool,
    pub http_stop: AtomicBool,
    pub backend_ready: AtomicBool,
    pub ocr_ready: AtomicBool,
    pub warmed: AtomicBool,
    pub next_request: AtomicU64,
    /// Upstream Host value, rendered once; `&'static Shared` lends it to
    /// `HeaderValue::from_static` without a per-request copy.
    pub upstream_host: String,
    slots: [Mutex<Option<Activity>>; MAX_ACTIVE],
    buffers: [Mutex<Option<BytesMut>>; MAX_ACTIVE],
    failure: Mutex<Option<&'static str>>,
    failure_reported: AtomicBool,
}

struct Activity {
    _lease: Lease<'static>,
    front: bool,
    backend: bool,
    input: bool,
}
pub(crate) struct Frontend {
    shared: &'static Shared,
    slot: usize,
    id: RequestId,
    started: std::time::Instant,
    outcome: &'static str,
}
pub(crate) struct Backend {
    shared: &'static Shared,
    slot: usize,
}
pub(crate) struct Upload {
    pub bytes: BytesMut,
    shared: &'static Shared,
    slot: usize,
}
enum Owner {
    Front,
    Backend,
    Input,
}

impl Shared {
    pub fn new(settings: Gateway) -> Self {
        let buffers = std::array::from_fn(|index| {
            Mutex::new(Some(BytesMut::with_capacity(
                if index < usize::from(settings.max_active) {
                    settings.max_request_bytes
                } else {
                    0
                },
            )))
        });
        Self {
            connections: Semaphore::new(usize::from(settings.max_connections)),
            ocr_ready: AtomicBool::new(settings.ocr_ready_endpoint.is_none()),
            upstream_host: settings.backend.to_string(),
            settings,
            buffers,
            lifecycle: Lifecycle::new(),
            changed: Notify::new(),
            java_stopped: AtomicBool::new(false),
            http_stop: AtomicBool::new(false),
            backend_ready: AtomicBool::new(false),
            warmed: AtomicBool::new(false),
            next_request: AtomicU64::new(request_seed()),
            slots: std::array::from_fn(|_| Mutex::new(None)),
            failure: Mutex::new(None),
            failure_reported: AtomicBool::new(false),
        }
    }
    pub fn request_id(&self) -> RequestId {
        RequestId::new(self.next_request.fetch_add(1, Ordering::Relaxed))
    }
    /// Admission, or the finite reason it was refused. The reason is a metric
    /// label and log field, never a document or peer detail.
    pub fn admit(&'static self) -> Result<(Frontend, Backend, Upload), &'static str> {
        if !self.backend_ready.load(Ordering::Acquire) {
            return Err("tika_unready");
        }
        if !self.ocr_ready.load(Ordering::Acquire) {
            return Err("ocr_unready");
        }
        let lease =
            self.lifecycle
                .admit(self.settings.max_active)
                .map_err(|error| match error {
                    AdmissionError::Saturated => "saturated",
                    AdmissionError::Unavailable(_) => "draining",
                })?;
        for (slot, storage) in self.slots.iter().enumerate() {
            let mut value = storage.lock().expect("activity mutex");
            if value.is_none() {
                let bytes = self.buffers[slot]
                    .lock()
                    .expect("buffer mutex")
                    .take()
                    .expect("free slot owns an input buffer");
                *value = Some(Activity {
                    _lease: lease,
                    front: true,
                    backend: true,
                    input: true,
                });
                indra_telemetry::gateway_admission(1);
                return Ok((
                    Frontend {
                        shared: self,
                        slot,
                        id: self.request_id(),
                        started: std::time::Instant::now(),
                        outcome: "client_cancelled",
                    },
                    Backend { shared: self, slot },
                    Upload {
                        bytes,
                        shared: self,
                        slot,
                    },
                ));
            }
        }
        self.fail("admission_invariant");
        Err("admission_invariant")
    }
    fn finish(&self, slot: usize, owner: Owner) {
        let mut storage = self.slots[slot].lock().expect("activity mutex");
        let value = storage.as_mut().expect("each owner drops exactly once");
        match owner {
            Owner::Front => value.front = false,
            Owner::Backend => value.backend = false,
            Owner::Input => value.input = false,
        }
        if !value.front && !value.backend && !value.input {
            drop(storage.take());
            indra_telemetry::gateway_admission(-1);
            self.changed.notify_one();
        }
    }
    pub fn drain(&self) {
        let state = self.lifecycle.drain();
        tracing::info!(
            event = "gateway_drain",
            phase = state.phase.as_str(),
            active = state.active
        );
        self.changed.notify_one();
    }
    pub fn fail(&self, class: &'static str) {
        self.lifecycle.fail();
        let mut failure = self.failure.lock().expect("failure mutex");
        if failure.is_none() {
            *failure = Some(class);
        }
        self.changed.notify_waiters();
    }
    pub fn report(&self, class: &'static str, request_id: Option<&str>, elapsed: Duration) {
        if !self.failure_reported.swap(true, Ordering::AcqRel) {
            indra_telemetry::report_failure(&indra_telemetry::FailureContext {
                class,
                phase: indra_telemetry::FailurePhase::Gateway,
                request_id,
                worker: None,
                elapsed,
            });
        }
    }
    /// Publish one probe round through each dependency's timeout debounce and
    /// log every change. Returns the OCR readiness now in effect.
    fn publish(
        &self,
        tika: &mut Probed,
        tika_result: &Result<bool, ProxyError>,
        ocr: &mut Probed,
        ocr_result: &Result<bool, ProxyError>,
    ) -> bool {
        let ocr_ready = ocr.observe(ocr_result);
        if self.ocr_ready.swap(ocr_ready, Ordering::AcqRel) != ocr_ready {
            tracing::info!(
                event = "ocr_dependency_changed",
                ready = ocr_ready,
                "OCR readiness changed; admission follows the dependency"
            );
        }
        let tika_ready = tika.observe(tika_result);
        if self.backend_ready.swap(tika_ready, Ordering::AcqRel) != tika_ready {
            tracing::info!(
                event = "tika_dependency_changed",
                ready = tika_ready,
                "Tika readiness changed; admission follows the dependency"
            );
        }
        ocr_ready
    }
    pub async fn stopped(&self) {
        loop {
            let changed = self.changed.notified();
            if self.java_stopped.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
}

/// Consecutive probe timeouts after which a dependency that answered before
/// counts as lost: about three seconds at one probe per second.
static PROBE_TIMEOUTS: u8 = 3;

/// Debounced readiness of one probed dependency. A slow answer under CPU
/// contention is not an outage, so isolated timeouts keep the last state. A
/// refused connection or a wrong answer withdraws readiness at once: work
/// admitted to a Tika that refuses connections would fail terminally.
struct Probed {
    ready: bool,
    timeouts: u8,
}

impl Probed {
    const fn new(ready: bool) -> Self {
        Self { ready, timeouts: 0 }
    }
    const fn observe(&mut self, result: &Result<bool, ProxyError>) -> bool {
        self.ready = match result {
            Ok(ready) => {
                self.timeouts = 0;
                *ready
            }
            Err(ProxyError::Deadline) if self.ready && self.timeouts + 1 < PROBE_TIMEOUTS => {
                self.timeouts += 1;
                true
            }
            Err(_) => {
                self.timeouts = 0;
                false
            }
        };
        self.ready
    }
}

/// Since when the OCR dependency of a Ready gateway has been unready.
struct OcrLoss(Option<Instant>);

impl OcrLoss {
    /// Whether the dependency has now been unready for at least `bound`.
    fn exceeded(&mut self, ready: bool, now: Instant, bound: Duration) -> bool {
        if ready {
            self.0 = None;
            return false;
        }
        now.saturating_duration_since(*self.0.get_or_insert(now)) >= bound
    }
}

/// Request IDs count up from the startup time in nanoseconds since the UNIX
/// epoch. A restarted or second gateway therefore does not repeat earlier IDs
/// unless it served more than one request per nanosecond since that start.
fn request_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos() as u64)
}

/// Correlation only, never a secret: 16 lowercase hex digits held inline, so
/// spans, failure reports and the response header borrow one stack value.
#[derive(Clone, Copy)]
pub(crate) struct RequestId([u8; 16]);

impl RequestId {
    const fn new(mut value: u64) -> Self {
        let mut hex = [b'0'; 16];
        let mut index = hex.len();
        while index > 0 {
            index -= 1;
            let digit = (value & 15) as u8;
            hex[index] = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + digit - 10
            };
            value >>= 4;
        }
        Self(hex)
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("generated request ID is lowercase ASCII hex")
    }
    /// The response header owns its bytes; this is its single bounded copy.
    pub fn header(&self) -> HeaderValue {
        HeaderValue::from_bytes(&self.0).expect("hex request ID is a valid header value")
    }
}

/// One JSON record and one metric sample for every proxied public request,
/// refused or admitted. The ID is an explicit field because Hyper often drops
/// the final response owner after the request span has closed.
pub(crate) fn request_finished(id: RequestId, outcome: &'static str, elapsed: Duration) {
    tracing::info!(
        event = "gateway_request_finished",
        request_id = id.as_str(),
        outcome,
        elapsed_ms = elapsed.as_secs_f64() * 1000.0,
        "Gateway request finished"
    );
    indra_telemetry::gateway_request_finished(outcome, elapsed);
}

impl Drop for Frontend {
    fn drop(&mut self) {
        request_finished(self.id, self.outcome, self.started.elapsed());
        self.shared.finish(self.slot, Owner::Front);
    }
}
impl Frontend {
    pub const fn id(&self) -> RequestId {
        self.id
    }
    pub fn record(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.shared.finish(self.slot, Owner::Backend);
    }
}
impl AsRef<[u8]> for Upload {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Drop for Upload {
    fn drop(&mut self) {
        self.bytes.clear();
        let buffer = std::mem::take(&mut self.bytes);
        *self.shared.buffers[self.slot].lock().expect("buffer mutex") = Some(buffer);
        self.shared.finish(self.slot, Owner::Input);
    }
}

#[derive(Serialize, ToSchema)]
pub struct Status {
    phase: &'static str,
    active: u16,
    capacity: u16,
    backend_ready: bool,
    ocr_ready: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct ApiError {
    pub code: &'static str,
    pub message: &'static str,
}

pub(crate) fn close(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(crate) fn api_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    close((status, Json(ApiError { code, message })).into_response())
}

#[derive(Clone, Copy)]
enum Probe {
    Ready,
    Health,
    Startup,
}

fn status(shared: &Shared, probe: Probe) -> Response {
    let snapshot = shared.lifecycle.snapshot();
    let backend_ready = shared.backend_ready.load(Ordering::Acquire);
    let ocr_ready = shared.ocr_ready.load(Ordering::Acquire);
    let alive = !matches!(snapshot.phase, Phase::Failed | Phase::Stopped);
    let okay = match probe {
        Probe::Ready => snapshot.phase == Phase::Ready && backend_ready && ocr_ready,
        Probe::Health => alive,
        Probe::Startup => shared.warmed.load(Ordering::Acquire) && alive,
    };
    close(
        (
            if okay {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            Json(Status {
                phase: snapshot.phase.as_str(),
                active: snapshot.active,
                capacity: shared.settings.max_active,
                backend_ready,
                ocr_ready: shared.settings.ocr_ready_endpoint.map(|_| ocr_ready),
            }),
        )
            .into_response(),
    )
}

#[utoipa::path(get, path="/readyz", responses((status=200, description="Tika version verified, optional OCR dependency ready and admission open; temporary parse saturation does not remove readiness", body=Status), (status=503, description="Starting, dependency unavailable, restarting, draining or failed", body=Status)))]
async fn ready(State(shared): State<&'static Shared>) -> Response {
    status(shared, Probe::Ready)
}
#[utoipa::path(get, path="/healthz", responses((status=200, description="Supervisor healthy, including startup and normal drain", body=Status), (status=503, description="Terminal failure", body=Status)))]
async fn health(State(shared): State<&'static Shared>) -> Response {
    status(shared, Probe::Health)
}
#[utoipa::path(get, path="/startupz", responses((status=200, description="Tika 3.3.2 startup completed", body=Status), (status=503, description="Not started or failed", body=Status)))]
async fn startup(State(shared): State<&'static Shared>) -> Response {
    status(shared, Probe::Startup)
}
#[utoipa::path(post, path="/drain", responses((status=200, description="Stop admission and finish accepted parses before signaling Java", body=Status), (status=403, description="Only an actual IPv4 loopback peer may drain", body=ApiError)))]
async fn drain(
    State(shared): State<&'static Shared>,
    ConnectInfo(peer): ConnectInfo<Peer>,
) -> Response {
    if !peer.0.is_ipv4() || !peer.0.ip().is_loopback() {
        return api_error(
            StatusCode::FORBIDDEN,
            "loopback_required",
            "Drain is restricted to IPv4 loopback socket peers",
        );
    }
    shared.drain();
    status(shared, Probe::Health)
}

async fn openapi() -> Response {
    close(Json(api_document()).into_response())
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Tika 3.3.2 admission gateway",
        description = "Operational API. Other paths proxy the pinned Tika API under bounded admission, upload size and deadlines; see docs/tika-gateway.md. Unsafe Tika endpoints remain disabled in its own configuration."
    ),
    paths(ready, health, startup, drain, proxy::handle),
    components(schemas(Status, ApiError, proxy::RawDocument))
)]
struct Api;
pub fn api_document() -> utoipa::openapi::OpenApi {
    Api::openapi()
}

pub(crate) fn router(shared: &'static Shared) -> Router {
    Router::new()
        .route("/readyz", get(ready))
        .route("/healthz", get(health))
        .route("/startupz", get(startup))
        .route("/drain", post(drain))
        .route("/openapi.json", get(openapi))
        .fallback(proxy::handle)
        .with_state(shared)
}

pub async fn serve(settings: Settings) -> Result<(), Error> {
    SHARED
        .set(Shared::new(settings.gateway))
        .map_err(|_| Error::Configuration("gateway can only initialize once"))?;
    let shared = SHARED.get().expect("initialized shared state");
    let listener = TcpListener::bind(shared.settings.listen)
        .await
        .map_err(|source| error::io("binding public HTTP listener", source))?;
    // Install handlers before Java starts, including during startup verification.
    let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|source| error::io("installing SIGTERM handler", source))?;
    let interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|source| error::io("installing SIGINT handler", source))?;
    let java = Java::spawn(&shared.settings)?;
    let server = axum::serve(
        LimitedListener {
            socket: listener,
            shared,
        },
        router(shared).into_make_service_with_connect_info::<Peer>(),
    )
    .with_graceful_shutdown(async {
        loop {
            let changed = shared.changed.notified();
            if shared.http_stop.load(Ordering::Acquire) {
                break;
            }
            changed.await;
        }
    });
    let mut server = tokio::spawn(async move { server.await });
    let supervised = supervise(shared, java, terminate, interrupt).await;
    shared.http_stop.store(true, Ordering::Release);
    shared.changed.notify_waiters();
    let served = match tokio::time::timeout(
        Duration::from_secs(shared.settings.client_stall_seconds + 1),
        &mut server,
    )
    .await
    {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(source))) => Err(error::io("serving HTTP", source)),
        Ok(Err(_)) => Err(Error::Failed("http_task")),
        Err(_) => {
            server.abort();
            let _ = server.await;
            Err(Error::Failed("http_shutdown_deadline"))
        }
    };
    supervised?;
    served
}

async fn supervise(
    shared: &'static Shared,
    mut java: Java,
    mut terminate: tokio::signal::unix::Signal,
    mut interrupt: tokio::signal::unix::Signal,
) -> Result<(), Error> {
    let startup_due = Instant::now() + Duration::from_secs(shared.settings.startup_timeout_seconds);
    let mut drain_due = None;
    let mut next_probe = Instant::now();
    let mut tika_probe = Probed::new(false);
    let mut ocr_probe = Probed::new(shared.settings.ocr_ready_endpoint.is_none());
    let mut ocr_loss = OcrLoss(None);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            _ = terminate.recv() => shared.drain(),
            _ = interrupt.recv() => shared.drain(),
            _ = shared.changed.notified() => {},
            _ = tick.tick() => {},
        }
        match java.reap() {
            Ok(_) => {}
            Err(_) => shared.fail("child_reap"),
        }
        if java.main_exit.is_some() {
            shared.fail("java_exited");
        }
        let phase = shared.lifecycle.snapshot();
        if phase.phase == Phase::Failed {
            break;
        }
        if phase.phase == Phase::Draining {
            let due = drain_due.get_or_insert_with(|| {
                Instant::now() + Duration::from_secs(shared.settings.shutdown_timeout_seconds)
            });
            if phase.active == 0 {
                break;
            }
            if Instant::now() >= *due {
                shared.fail("drain_deadline");
                break;
            }
            continue;
        }
        if Instant::now() >= next_probe {
            let (result, ocr) = tokio::join!(proxy::version(shared.settings.backend), async {
                match shared.settings.ocr_ready_endpoint {
                    Some(address) => proxy::ocr_ready(address).await,
                    None => Ok(true),
                }
            },);
            next_probe = Instant::now() + Duration::from_secs(1);
            let ocr_ready = shared.publish(&mut tika_probe, &result, &mut ocr_probe, &ocr);
            // In Compose the OCR container owns the network namespace. After it
            // restarts, this process still sits in the old one, where OCR never
            // returns; exiting lets the restart policy join the current one.
            if phase.phase == Phase::Ready
                && let Some(seconds) = shared.settings.ocr_loss_exit_seconds
                && ocr_loss.exceeded(ocr_ready, Instant::now(), Duration::from_secs(seconds))
            {
                shared.fail("ocr_dependency_lost");
                break;
            }
            match result {
                Ok(false) => {
                    shared.fail("wrong_tika_version");
                    break;
                }
                Ok(true)
                    if ocr_ready
                        && phase.phase == Phase::Starting
                        && shared.lifecycle.activate().is_ok() =>
                {
                    shared.warmed.store(true, Ordering::Release);
                    tracing::info!(event = "gateway_ready", tika_version = "3.3.2");
                }
                _ => {}
            }
        }
        if phase.phase == Phase::Starting && Instant::now() >= startup_due {
            shared.fail("startup_deadline");
            break;
        }
    }
    shared.backend_ready.store(false, Ordering::Release);
    let termination = java.terminate(shared.settings.child_shutdown_seconds).await;
    if termination.is_err() {
        shared.fail("child_termination");
    }
    shared
        .java_stopped
        .store(termination.is_ok(), Ordering::Release);
    shared.changed.notify_waiters();
    let failure = *shared.failure.lock().expect("failure mutex");
    if let Some(class) = failure {
        shared.report(class, None, Duration::ZERO);
        return Err(Error::Failed(class));
    }
    termination?;
    shared
        .lifecycle
        .stop()
        .map_err(|_| Error::Failed("stop_with_active_work"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    // One state per test that changes it: libtest runs tests in parallel.
    static CASES: [OnceLock<Shared>; 7] = [const { OnceLock::new() }; 7];
    pub fn shared(index: usize) -> &'static Shared {
        CASES[index].get_or_init(|| {
            let shared = Shared::new(Gateway {
                max_active: 2,
                max_request_bytes: 1024,
                max_response_bytes: 1024,
                ..Gateway::default()
            });
            shared.lifecycle.activate().expect("start test state");
            shared.backend_ready.store(true, Ordering::Release);
            shared.warmed.store(true, Ordering::Release);
            shared
        })
    }

    #[test]
    fn request_ids_continue_from_the_startup_time() {
        let unix = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after the epoch")
                .as_nanos() as u64
        };
        let before = unix();
        let shared = Shared::new(Gateway {
            max_active: 1,
            max_request_bytes: 1,
            ..Gateway::default()
        });
        let after = unix();
        // A restart or a second replica starts from its own later instant
        // instead of 1, so its IDs do not repeat those already returned.
        let first = u64::from_str_radix(shared.request_id().as_str(), 16).expect("hex ID");
        assert!(
            (before..=after).contains(&first),
            "{before} <= {first} <= {after}"
        );
    }

    #[test]
    fn request_ids_render_as_fixed_width_lowercase_hex() {
        for (value, text) in [
            (1, "0000000000000001"),
            (0x0123_4567_89ab_cdef, "0123456789abcdef"),
            (u64::MAX, "ffffffffffffffff"),
        ] {
            let id = RequestId::new(value);
            assert_eq!(id.as_str(), text);
            assert_eq!(id.header(), text);
        }
    }

    #[tokio::test]
    async fn ocr_dependency_blocks_admission_without_failing_supervisor_health() {
        let shared = shared(4);
        shared.ocr_ready.store(false, Ordering::Release);
        assert_eq!(shared.admit().err(), Some("ocr_unready"));
        for (path, expected) in [
            ("/readyz", StatusCode::SERVICE_UNAVAILABLE),
            ("/healthz", StatusCode::OK),
            ("/startupz", StatusCode::OK),
        ] {
            let response = router(shared)
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        shared.ocr_ready.store(true, Ordering::Release);
        drop(shared.admit().expect("dependency recovered"));
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[tokio::test]
    async fn probe_timeouts_withdraw_readiness_only_when_they_persist() {
        let shared = shared(6);
        let ready = || async move {
            router(shared)
                .oneshot(
                    Request::builder()
                        .uri("/readyz")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        };
        let answer: Result<bool, ProxyError> = Ok(true);
        let timeout: Result<bool, ProxyError> = Err(ProxyError::Deadline);
        let refused: Result<bool, ProxyError> = Err(ProxyError::Upstream);
        let (mut tika, mut ocr) = (Probed::new(true), Probed::new(true));
        // One or two slow answers, e.g. a GC pause under the shared CPU
        // quota, keep readiness and admission.
        for _ in 0..2 {
            assert!(shared.publish(&mut tika, &timeout, &mut ocr, &timeout));
            assert_eq!(ready().await, StatusCode::OK);
            drop(shared.admit().expect("admission stays open"));
        }
        // The third consecutive timeout is an outage, and further timeouts
        // cannot restore what only an answer proves.
        for _ in 0..2 {
            assert!(!shared.publish(&mut tika, &timeout, &mut ocr, &timeout));
            assert_eq!(ready().await, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(shared.admit().err(), Some("tika_unready"));
        }
        assert!(shared.publish(&mut tika, &answer, &mut ocr, &answer));
        assert_eq!(ready().await, StatusCode::OK);
        // A refused connection or a draining sidecar's 503 withdraws at once.
        assert!(shared.publish(&mut tika, &refused, &mut ocr, &answer));
        assert_eq!(shared.admit().err(), Some("tika_unready"));
        assert!(!shared.publish(&mut tika, &answer, &mut ocr, &refused));
        assert_eq!(shared.admit().err(), Some("ocr_unready"));
        assert!(shared.publish(&mut tika, &answer, &mut ocr, &answer));
        assert_eq!(ready().await, StatusCode::OK);
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[test]
    fn ocr_loss_counts_only_an_uninterrupted_outage() {
        let bound = Duration::from_secs(30);
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut loss = OcrLoss(None);
        assert!(!loss.exceeded(false, at(0), bound));
        assert!(!loss.exceeded(false, at(29), bound));
        // One ready answer restarts the count.
        assert!(!loss.exceeded(true, at(30), bound));
        assert!(!loss.exceeded(false, at(31), bound));
        assert!(!loss.exceeded(false, at(60), bound));
        assert!(loss.exceeded(false, at(61), bound));
    }

    #[tokio::test]
    async fn refused_requests_answer_503_with_retry_after_before_any_upload() {
        let shared = shared(5);
        shared.ocr_ready.store(false, Ordering::Release);
        let response = router(shared)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/tika")
                    .header(header::HOST, "localhost")
                    .header(header::CONTENT_LENGTH, "4")
                    .body(Body::from("page"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .map(|v| v.to_str().unwrap()),
            Some("1")
        );
        assert_eq!(response.headers().get("x-request-id").unwrap().len(), 16);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "unavailable");
        assert_eq!(shared.lifecycle.snapshot().active, 0);
        shared.ocr_ready.store(true, Ordering::Release);
    }

    #[tokio::test]
    async fn readiness_ignores_saturation_and_drain_rejects_new_work() {
        let shared = shared(0);
        let first = shared.admit().expect("first");
        let second = shared.admit().expect("second");
        assert_eq!(shared.admit().err(), Some("saturated"));
        let ready = router(shared)
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::OK);
        shared.drain();
        assert_eq!(shared.admit().err(), Some("draining"));
        let ready = router(shared)
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
        let health = router(shared)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        drop(first);
        assert_eq!(shared.lifecycle.snapshot().active, 1);
        drop(second);
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }

    #[tokio::test]
    async fn actual_peer_controls_drain_and_upload_errors_release_all_owners() {
        let shared = shared(1);
        let mut request = Request::builder()
            .method("POST")
            .uri("/drain")
            .body(Body::empty())
            .unwrap();
        request
            .headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static("127.0.0.1"));
        request
            .extensions_mut()
            .insert(ConnectInfo(Peer("10.0.0.2:4321".parse().unwrap())));
        let denied = router(shared).oneshot(request).await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert_eq!(shared.lifecycle.snapshot().phase, Phase::Ready);
        let request = Request::builder()
            .method("PUT")
            .uri("/tika")
            .header("content-length", "4")
            .body(Body::from("abc"))
            .unwrap();
        let rejected = router(shared).oneshot(request).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let message = to_bytes(rejected.into_body(), 1024).await.unwrap();
        assert!(
            std::str::from_utf8(&message)
                .unwrap()
                .contains("invalid_upload")
        );
        assert_eq!(shared.lifecycle.snapshot().active, 0);
    }
}
