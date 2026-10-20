//! Finite public failures; native diagnostics never become response text.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;
use utoipa::ToSchema;

use crate::protocol::RequestId;

#[derive(Debug, Error, Diagnostic)]
pub enum StartupError {
    #[error("configuration {stage} failed at {field}: {problem}")]
    #[diagnostic(
        code(indra::configuration),
        help(
            "Configuration values are redacted. Use docs/runtime-service.md and indra-server check-config."
        )
    )]
    Configuration {
        stage: &'static str,
        field: String,
        problem: &'static str,
    },
    #[error("invalid configuration: {0}")]
    #[diagnostic(
        code(indra::invalid_config),
        help(
            "Run `indra-server check-config`; limits and precedence are in docs/runtime-service.md."
        )
    )]
    InvalidConfig(&'static str),
    #[error("debug .env exists but could not be read or parsed")]
    #[diagnostic(
        code(indra::dotenv),
        help("Only debug builds read ./.env; fix or remove the file. Release binaries ignore it.")
    )]
    Dotenv,
    #[error("model verification failed")]
    #[diagnostic(
        code(indra::models),
        help(
            "Run `indra-server models verify` or `models fetch` with the same --math setting and model directory."
        )
    )]
    Models(#[source] indra_models::CacheError),
    #[error("telemetry initialization or shutdown failed")]
    #[diagnostic(
        code(indra::telemetry),
        help(
            "Check the OTLP endpoint/authorization and Sentry DSN settings; values are never printed."
        )
    )]
    Telemetry(#[source] indra_telemetry::TelemetryError),
    #[error("server I/O failed")]
    #[diagnostic(code(indra::io))]
    Io(#[from] std::io::Error),
    #[error("OCR worker could not start")]
    #[diagnostic(
        code(indra::worker),
        help(
            "The JSON log carries the finite failure class; verify the model cache and native libraries."
        )
    )]
    Worker,
    #[error("OCR startup deadline expired")]
    #[diagnostic(
        code(indra::startup_deadline),
        help("Raise startup_timeout_seconds or give the container more CPU for model warmup.")
    )]
    StartupDeadline,
    #[error("OCR service stopped after a terminal failure: {0}")]
    #[diagnostic(
        code(indra::terminal),
        help(
            "See the service_terminal JSON event; the orchestrator restart policy recovers the service."
        )
    )]
    Terminal(&'static str),
    #[error("accepted work or response ownership exceeded the shutdown deadline")]
    #[diagnostic(
        code(indra::drain_deadline),
        help("Raise shutdown_timeout_seconds and the deployment termination grace together.")
    )]
    DrainDeadline,
    #[error("OCR service has already been initialized in this process")]
    #[diagnostic(code(indra::already_initialized))]
    AlreadyInitialized,
    #[error("local HTTP probe failed")]
    #[diagnostic(
        code(indra::probe),
        help("The endpoint answered a non-200 status or did not answer within two seconds.")
    )]
    Probe,
    #[error("could not serialize the OpenAPI document")]
    #[diagnostic(code(indra::openapi))]
    OpenApi,
}

impl From<indra_models::CacheError> for StartupError {
    fn from(error: indra_models::CacheError) -> Self {
        Self::Models(error)
    }
}

impl From<indra_telemetry::TelemetryError> for StartupError {
    fn from(error: indra_telemetry::TelemetryError) -> Self {
        Self::Telemetry(error)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Forbidden,
    LengthRequired,
    TooLarge,
    UnsupportedMediaType,
    InvalidImage,
    QueueFull,
    Unavailable,
    Deadline,
    NativeFailure,
    Internal,
}

impl ErrorCode {
    pub const fn status(self) -> StatusCode {
        match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::LengthRequired => StatusCode::LENGTH_REQUIRED,
            Self::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::InvalidImage => StatusCode::UNPROCESSABLE_ENTITY,
            Self::QueueFull | Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Deadline => StatusCode::GATEWAY_TIMEOUT,
            Self::NativeFailure | Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Forbidden => "forbidden",
            Self::LengthRequired => "length_required",
            Self::TooLarge => "too_large",
            Self::UnsupportedMediaType => "unsupported_media_type",
            Self::InvalidImage => "invalid_image",
            Self::QueueFull => "queue_full",
            Self::Unavailable => "unavailable",
            Self::Deadline => "deadline",
            Self::NativeFailure => "native_failure",
            Self::Internal => "internal",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request framing or body length is invalid.",
            Self::Forbidden => "The drain endpoint is available only to loopback peers.",
            Self::LengthRequired => "Supply one positive Content-Length header.",
            Self::TooLarge => "The configured input, decoded-image, or output limit was exceeded.",
            Self::UnsupportedMediaType => "Send an encoded PNG, JPEG, or TIFF image.",
            Self::InvalidImage => "The encoded image is invalid or unsupported.",
            Self::QueueFull => "All admitted request slots are occupied; retry later.",
            Self::Unavailable => "OCR is starting, draining, or unavailable; retry later.",
            Self::Deadline => "The OCR request deadline expired.",
            Self::NativeFailure => "OCR failed; the service is restarting.",
            Self::Internal => "An internal ownership or worker failure occurred.",
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse<'a> {
    pub code: ErrorCode,
    pub message: &'static str,
    #[schema(min_length = 32, max_length = 32, pattern = "^[0-9a-f]{32}$")]
    pub request_id: &'a str,
}

pub fn response(code: ErrorCode, request_id: &RequestId) -> Response {
    let mut response = (
        code.status(),
        Json(ErrorResponse {
            code,
            message: code.message(),
            request_id: request_id.as_str(),
        }),
    )
        .into_response();
    crate::protocol::set_common_headers(response.headers_mut(), request_id);
    if matches!(code, ErrorCode::QueueFull | ErrorCode::Unavailable) {
        response.headers_mut().insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("1"),
        );
    }
    response
}
