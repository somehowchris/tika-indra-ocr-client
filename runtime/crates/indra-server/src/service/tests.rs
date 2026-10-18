use super::{
    http,
    state::{Completed, Shared, Work},
};
use crate::{configuration::Settings, error::ErrorCode};
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use indra_core::lifecycle::Phase;
use indra_engine::OcrStats;
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{OnceLock, atomic::Ordering},
    time::Duration,
};
use tower::ServiceExt;

static STATE: OnceLock<Shared> = OnceLock::new();

fn image() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/ocr")
        .header("content-length", "5")
        .header("content-type", "application/octet-stream")
        .body(Body::from("image"))
        .expect("image request")
}

async fn queued(shared: &'static Shared) -> Work {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(work) = shared.queue.lock().expect("queue").pop() {
                break work;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("request entered FIFO")
}

async fn probe(shared: &'static Shared, path: &'static str) -> StatusCode {
    http::router(shared)
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("probe"),
        )
        .await
        .expect("response")
        .status()
}

#[tokio::test]
async fn overload_timeout_disconnect_and_drain_preserve_native_and_response_ownership() {
    let shared = STATE.get_or_init(|| {
        let mut settings = Settings::default();
        settings.server.queue_capacity = 1;
        settings.server.max_request_bytes = 16;
        settings.server.max_response_bytes = 16;
        settings.server.request_timeout_seconds = 1;
        settings.server.upload_timeout_seconds = 1;
        Shared::new(settings)
    });
    assert_eq!(
        probe(shared, "/readyz").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        probe(shared, "/startupz").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(probe(shared, "/healthz").await, StatusCode::OK);
    assert_eq!(
        http::router(shared)
            .oneshot(image())
            .await
            .expect("starting")
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    shared.lifecycle.activate().expect("warm activation");
    shared.warmed.store(true, Ordering::Release);
    shared.ready_workers.store(1, Ordering::Release);

    let first = shared.reserve().expect("first admitted slot");
    let second = shared.reserve().expect("second admitted slot");
    assert_eq!(
        probe(shared, "/readyz").await,
        StatusCode::OK,
        "saturation does not change readiness"
    );
    let rejected = http::router(shared)
        .oneshot(image())
        .await
        .expect("overload");
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(rejected.headers()["retry-after"], "1");
    let rejected = rejected
        .into_body()
        .collect()
        .await
        .expect("error body")
        .to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&rejected).expect("typed JSON error");
    assert_eq!(error["code"], "queue_full");
    assert_eq!(error["request_id"].as_str().expect("ID").len(), 32);
    drop(first);
    drop(second);

    let request = tokio::spawn(async move { http::router(shared).oneshot(image()).await });
    let work = queued(shared).await;
    request.abort();
    assert!(request.await.is_err());
    assert!(work.response.is_closed());
    assert_eq!(
        shared.lifecycle.snapshot().active,
        1,
        "HTTP cancellation cannot release a native-owned lease"
    );
    drop(work);
    assert_eq!(shared.lifecycle.snapshot().active, 0);

    let request = tokio::spawn(async move { http::router(shared).oneshot(image()).await });
    let work = queued(shared).await;
    let expired = request
        .await
        .expect("request task")
        .expect("deadline response");
    assert_eq!(expired.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        shared.lifecycle.snapshot().active,
        1,
        "timeout keeps retained work accounted"
    );
    drop(work);
    assert_eq!(shared.lifecycle.snapshot().active, 0);

    let remote_drain = Request::builder()
        .method("POST")
        .uri("/drain")
        .extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 1234))))
        .body(Body::empty())
        .expect("remote drain");
    assert_eq!(
        http::router(shared)
            .oneshot(remote_drain)
            .await
            .expect("forbidden")
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(shared.lifecycle.snapshot().phase, Phase::Ready);

    let request = tokio::spawn(async move { http::router(shared).oneshot(image()).await });
    let mut work = queued(shared).await;
    let drain = Request::builder()
        .method("POST")
        .uri("/drain")
        .extension(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 1234))))
        .body(Body::empty())
        .expect("local drain");
    assert_eq!(
        http::router(shared)
            .oneshot(drain)
            .await
            .expect("drain")
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        probe(shared, "/readyz").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(probe(shared, "/healthz").await, StatusCode::OK);
    assert_eq!(probe(shared, "/startupz").await, StatusCode::OK);
    assert!(matches!(shared.reserve(), Err(ErrorCode::Unavailable)));
    assert!(shared.lifecycle.stop().is_err());

    work.reservation.buffers_mut().text.push_str("result");
    let mut stats = OcrStats::new(&shared.engine, 1);
    stats.output_bytes = 6;
    assert!(
        work.response
            .send(Ok(Completed {
                reservation: work.reservation,
                stats
            }))
            .is_ok()
    );
    let response = request
        .await
        .expect("accepted task")
        .expect("successful accepted work");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(response.headers()["content-length"], "6");
    assert_eq!(
        shared.lifecycle.snapshot().active,
        1,
        "response storage still has a lease"
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    assert_eq!(&bytes[..], b"result");
    assert_eq!(
        shared.lifecycle.snapshot().active,
        1,
        "the final bytes reference retains the reusable buffer"
    );
    drop(bytes);
    assert_eq!(shared.lifecycle.snapshot().active, 0);
    shared
        .lifecycle
        .stop()
        .expect("all native work and response ownership finished");
}

#[test]
fn openapi_covers_public_failure_and_lifecycle_contracts() {
    let api = serde_json::to_value(http::document()).expect("serializable OpenAPI");
    for status in [
        "200", "400", "411", "413", "415", "422", "500", "503", "504",
    ] {
        assert!(
            api["paths"]["/ocr"]["post"]["responses"]
                .get(status)
                .is_some(),
            "documented {status}"
        );
    }
    for path in [
        "/healthz",
        "/readyz",
        "/startupz",
        "/drain",
        "/openapi.json",
    ] {
        assert!(api["paths"].get(path).is_some(), "documented {path}");
    }
    let media = &api["paths"]["/ocr"]["post"]["responses"]["200"]["content"];
    assert!(media.get("text/plain; charset=utf-8").is_some());
}

#[tokio::test]
async fn supervisor_watchdog_fails_without_releasing_native_owned_capacity() {
    static WATCHDOG_STATE: OnceLock<Shared> = OnceLock::new();
    let shared = WATCHDOG_STATE.get_or_init(|| {
        let mut settings = Settings::default();
        settings.server.max_request_bytes = 16;
        settings.server.native_timeout_grace_seconds = 1;
        Shared::new(settings)
    });
    shared.lifecycle.activate().expect("ready native worker");
    let reservation = shared.reserve().expect("native call owns admission");
    *shared.native_deadlines[0].lock().expect("native deadline") =
        Some(std::time::Instant::now() - Duration::from_secs(2));
    let (finish, native) = std::sync::mpsc::sync_channel::<()>(0);
    let worker = std::thread::spawn(move || {
        native.recv().expect("explicit native return");
        drop(reservation);
        Ok(())
    });
    let mut workers = [Some(worker), None];
    let result = super::supervise(
        shared,
        std::time::Instant::now() + Duration::from_secs(5),
        &workers,
    )
    .await;
    assert!(matches!(
        result,
        Err(crate::error::StartupError::Terminal(
            "native deadline watchdog"
        ))
    ));
    assert_eq!(shared.lifecycle.snapshot().phase, Phase::Failed);
    assert_eq!(
        shared.lifecycle.snapshot().active,
        1,
        "watchdog cannot free native-owned buffers"
    );
    finish
        .send(())
        .expect("allow the held native call to return");
    workers[0]
        .take()
        .expect("worker handle")
        .join()
        .expect("native thread")
        .expect("native result");
    assert_eq!(shared.lifecycle.snapshot().active, 0);
}
