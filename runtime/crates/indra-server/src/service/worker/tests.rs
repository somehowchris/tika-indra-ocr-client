use super::{finish, report_phase};
use crate::{
    configuration::Settings,
    protocol::RequestId,
    service::state::{Shared, Work},
};
use indra_engine::EngineError;
use std::{
    io,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tracing_subscriber::layer::SubscriberExt;

/// The JSON log lines written while a test's subscriber is the default.
struct Log(Mutex<([u8; 8192], usize)>);

impl io::Write for &Log {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut log = self.0.lock().expect("log");
        let (buffer, used) = &mut *log;
        let end = *used + bytes.len();
        buffer
            .get_mut(*used..end)
            .ok_or(io::ErrorKind::StorageFull)?
            .copy_from_slice(bytes);
        *used = end;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn engine_reason_reaches_the_rejection_log_and_the_failure_report() {
    static LOG: Log = Log(Mutex::new(([0; 8192], 0)));
    static STATE: OnceLock<Shared> = OnceLock::new();
    let shared = STATE.get_or_init(|| Shared::new(Settings::default()));
    shared.lifecycle.activate().expect("ready native worker");
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().json().with_writer(|| &LOG));
    tracing::subscriber::with_default(subscriber, || {
        let (response, _receiver) = tokio::sync::oneshot::channel();
        let now = Instant::now();
        let work = Work {
            reservation: shared.reserve().expect("admitted request"),
            response,
            request_id: RequestId::next(),
            deadline: now + Duration::from_secs(5),
            queued_at: now,
            span: tracing::Span::none(),
        };
        // A rejected input is a request error: logged, never reported.
        finish(
            shared,
            0,
            work,
            now,
            Err(EngineError::InvalidImage("truncated JPEG segment")),
        )
        .expect("a rejected image keeps the worker");
        report_phase(
            shared,
            0,
            None,
            Duration::ZERO,
            &EngineError::Shape("recognizer output dimensions"),
            indra_telemetry::FailurePhase::Inference,
        );
    });
    let log = LOG.0.lock().expect("log");
    let log = std::str::from_utf8(&log.0[..log.1]).expect("JSON lines");
    let line = |event: &str| log.lines().find(|line| line.contains(event));
    let rejected = line(r#""event":"inference_rejected""#).expect("rejection logged");
    assert!(
        rejected.contains(r#""failure_class":"invalid_image""#)
            && rejected.contains(r#""reason":"truncated JPEG segment""#),
        "{rejected}"
    );
    let failure = line(r#""event":"service_failure""#).expect("failure reported");
    assert!(
        failure.contains(r#""failure_class":"model_shape""#)
            && failure.contains(r#""reason":"recognizer output dimensions""#),
        "{failure}"
    );
}
