use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("invalid gateway setting: {0}")]
    #[diagnostic(code(indra::gateway::configuration))]
    Configuration(&'static str),
    #[error("invalid configuration field {field}: {problem}; source values are redacted")]
    #[diagnostic(code(indra::gateway::decoding))]
    Decoding {
        field: String,
        problem: &'static str,
    },
    #[error("I/O failed during {operation}: {source}")]
    #[diagnostic(code(indra::gateway::io))]
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    #[error("process supervision failed: {0}")]
    #[diagnostic(code(indra::gateway::process))]
    Process(&'static str),
    #[error("gateway stopped after terminal failure: {0}")]
    #[diagnostic(code(indra::gateway::failed))]
    Failed(&'static str),
    #[error("gateway probe did not return HTTP 200 within two seconds")]
    #[diagnostic(code(indra::gateway::probe))]
    Probe,
    #[error(transparent)]
    #[diagnostic(code(indra::gateway::telemetry))]
    Telemetry(#[from] indra_telemetry::TelemetryError),
}

pub fn io(operation: &'static str, source: std::io::Error) -> Error {
    Error::Io { operation, source }
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("request headers are invalid or exceed the 16 KiB bound")]
    Headers,
    #[error("a single bounded Content-Length is required for an upload")]
    Length,
    #[error("request body is invalid, truncated or has trailers")]
    Upload,
    #[error("request body exceeds the configured limit")]
    TooLarge,
    #[error("request deadline expired")]
    Deadline,
    #[error("Tika connection or response failed")]
    Upstream,
    #[error("Tika response exceeds the configured limit")]
    ResponseLimit,
    #[error("gateway is terminating the Tika process group")]
    Stopping,
}

impl ProxyError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Headers => "invalid_headers",
            Self::Length => "content_length_required",
            Self::Upload => "invalid_upload",
            Self::TooLarge => "request_too_large",
            Self::Deadline => "deadline_exceeded",
            Self::Upstream => "tika_failure",
            Self::ResponseLimit => "response_too_large",
            Self::Stopping => "terminating",
        }
    }
}
