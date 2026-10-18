use crate::error::Error;
use clap::{Args, Parser, Subcommand, ValueEnum};
use config::{Config, ConfigError, Environment, File, FileFormat};
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf};

pub const MAX_ACTIVE: usize = 8;
pub const MAX_HEADERS: usize = 16 * 1024;

#[derive(Parser)]
#[command(
    version,
    about = "Drain accepted Tika HTTP work before terminating its forked JVMs"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(flatten)]
    pub overrides: Overrides,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    Serve,
    CheckConfig,
    Openapi,
    Probe { kind: ProbeKind },
    Drain,
    Completions { shell: clap_complete::Shell },
}

// ValueEnum requires Clone; these are CLI-only discriminants, never payloads.
#[derive(Clone, ValueEnum)]
pub enum ProbeKind {
    Ready,
    Health,
    Startup,
}

#[derive(Default, Args)]
pub struct Overrides {
    #[arg(long, global = true)]
    pub listen: Option<SocketAddr>,
    #[arg(long, global = true)]
    pub backend: Option<SocketAddr>,
    #[arg(long, global = true)]
    pub ocr_ready_endpoint: Option<SocketAddr>,
    #[arg(long, global = true)]
    pub java: Option<String>,
    #[arg(long, global = true)]
    pub jar: Option<String>,
    #[arg(long, global = true)]
    pub tika_config: Option<String>,
    #[arg(long, global = true)]
    pub max_active: Option<u16>,
    #[arg(long, global = true)]
    pub max_connections: Option<u16>,
    #[arg(long, global = true)]
    pub max_request_bytes: Option<u64>,
    #[arg(long, global = true)]
    pub max_response_bytes: Option<u64>,
    #[arg(long, global = true)]
    pub header_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub upload_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub request_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub client_stall_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub startup_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub shutdown_timeout_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub child_shutdown_seconds: Option<u64>,
    #[arg(long, global = true)]
    pub log_filter: Option<String>,
    #[arg(long, global = true)]
    pub service_name: Option<String>,
    #[arg(long, global = true)]
    pub environment: Option<String>,
    #[arg(long, global = true)]
    pub otlp_endpoint: Option<String>,
    #[arg(long, global = true)]
    pub otlp_authorization: Option<String>,
    #[arg(long, global = true)]
    pub sentry_dsn: Option<String>,
    #[arg(long, global = true)]
    pub trace_sample_ratio: Option<f64>,
    #[arg(long, global = true)]
    pub export_queue_size: Option<u64>,
    #[arg(long, global = true)]
    pub export_batch_size: Option<u64>,
    #[arg(long, global = true)]
    pub export_timeout_ms: Option<u64>,
    #[arg(long, global = true)]
    pub export_interval_ms: Option<u64>,
    #[arg(long, global = true)]
    pub log_queue_size: Option<u64>,
    #[arg(long, global = true)]
    pub sentry_sample_ratio: Option<f64>,
    #[arg(long, global = true)]
    pub sentry_queue_size: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub gateway: Gateway,
    pub telemetry: indra_telemetry::Options,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Gateway {
    pub listen: SocketAddr,
    pub backend: SocketAddr,
    pub ocr_ready_endpoint: Option<SocketAddr>,
    pub java: PathBuf,
    pub jar: PathBuf,
    pub tika_config: PathBuf,
    pub max_active: u16,
    pub max_connections: u16,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub header_timeout_seconds: u64,
    pub upload_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub client_stall_seconds: u64,
    pub startup_timeout_seconds: u64,
    pub shutdown_timeout_seconds: u64,
    pub child_shutdown_seconds: u64,
}

impl Default for Gateway {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 9998)),
            backend: SocketAddr::from(([127, 0, 0, 1], 9999)),
            ocr_ready_endpoint: None,
            java: PathBuf::from("/usr/bin/java"),
            jar: PathBuf::from("/opt/tika/tika-server.jar"),
            tika_config: PathBuf::from("/etc/tika/tika-config.xml"),
            max_active: 2,
            max_connections: 16,
            max_request_bytes: 64 * 1024 * 1024,
            max_response_bytes: 16 * 1024 * 1024,
            header_timeout_seconds: 5,
            upload_timeout_seconds: 15,
            request_timeout_seconds: 300,
            client_stall_seconds: 5,
            startup_timeout_seconds: 180,
            shutdown_timeout_seconds: 315,
            child_shutdown_seconds: 5,
        }
    }
}

impl Settings {
    pub fn load(file: Option<PathBuf>, overrides: Overrides) -> Result<Self, Error> {
        Self::from_sources(
            file,
            overrides,
            Environment::with_prefix("TIKA")
                .prefix_separator("__")
                .separator("__")
                .try_parsing(true),
        )
    }

    fn from_sources(
        file: Option<PathBuf>,
        overrides: Overrides,
        env: Environment,
    ) -> Result<Self, Error> {
        let mut builder = Config::builder()
            .set_default("telemetry.service_name", "tika-gateway")
            .map_err(decode)?;
        if let Some(file) = file {
            builder = builder.add_source(File::from(file).format(FileFormat::Toml).required(true));
        }
        builder = builder.add_source(env);
        macro_rules! set {
            ($key:literal, $value:expr) => {
                if let Some(value) = $value {
                    builder = builder.set_override($key, value).map_err(decode)?;
                }
            };
        }
        set!("gateway.listen", overrides.listen.map(|v| v.to_string()));
        set!("gateway.backend", overrides.backend.map(|v| v.to_string()));
        set!(
            "gateway.ocr_ready_endpoint",
            overrides.ocr_ready_endpoint.map(|v| v.to_string())
        );
        set!("gateway.java", overrides.java);
        set!("gateway.jar", overrides.jar);
        set!("gateway.tika_config", overrides.tika_config);
        set!("gateway.max_active", overrides.max_active);
        set!("gateway.max_connections", overrides.max_connections);
        set!("gateway.max_request_bytes", overrides.max_request_bytes);
        set!("gateway.max_response_bytes", overrides.max_response_bytes);
        set!(
            "gateway.header_timeout_seconds",
            overrides.header_timeout_seconds
        );
        set!(
            "gateway.upload_timeout_seconds",
            overrides.upload_timeout_seconds
        );
        set!(
            "gateway.request_timeout_seconds",
            overrides.request_timeout_seconds
        );
        set!(
            "gateway.client_stall_seconds",
            overrides.client_stall_seconds
        );
        set!(
            "gateway.startup_timeout_seconds",
            overrides.startup_timeout_seconds
        );
        set!(
            "gateway.shutdown_timeout_seconds",
            overrides.shutdown_timeout_seconds
        );
        set!(
            "gateway.child_shutdown_seconds",
            overrides.child_shutdown_seconds
        );
        set!("telemetry.log_filter", overrides.log_filter);
        set!("telemetry.service_name", overrides.service_name);
        set!("telemetry.environment", overrides.environment);
        set!("telemetry.otlp_endpoint", overrides.otlp_endpoint);
        set!("telemetry.otlp_authorization", overrides.otlp_authorization);
        set!("telemetry.sentry_dsn", overrides.sentry_dsn);
        set!("telemetry.trace_sample_ratio", overrides.trace_sample_ratio);
        set!("telemetry.export_queue_size", overrides.export_queue_size);
        set!("telemetry.export_batch_size", overrides.export_batch_size);
        set!("telemetry.export_timeout_ms", overrides.export_timeout_ms);
        set!("telemetry.export_interval_ms", overrides.export_interval_ms);
        set!("telemetry.log_queue_size", overrides.log_queue_size);
        set!(
            "telemetry.sentry_sample_ratio",
            overrides.sentry_sample_ratio
        );
        set!("telemetry.sentry_queue_size", overrides.sentry_queue_size);
        let settings: Self = builder
            .build()
            .map_err(decode)?
            .try_deserialize()
            .map_err(decode)?;
        settings.gateway.validate()?;
        settings.telemetry.validate()?;
        Ok(settings)
    }
}

impl Gateway {
    pub fn validate(&self) -> Result<(), Error> {
        if !self.listen.is_ipv4()
            || self.listen.port() == 0
            || !self.backend.is_ipv4()
            || !self.backend.ip().is_loopback()
            || self.backend.port() == 0
            || self.listen.port() == self.backend.port()
        {
            return Err(Error::Configuration(
                "listen must be IPv4; backend must be IPv4 loopback with a different nonzero port",
            ));
        }
        if self.ocr_ready_endpoint.is_some_and(|address| {
            !address.is_ipv4()
                || !address.ip().is_loopback()
                || address.port() == 0
                || address.port() == self.listen.port()
                || address.port() == self.backend.port()
        }) {
            return Err(Error::Configuration(
                "ocr_ready_endpoint must be IPv4 loopback on its own nonzero port",
            ));
        }
        if !(1..=MAX_ACTIVE as u16).contains(&self.max_active)
            || !(self.max_active + 4..=64).contains(&self.max_connections)
        {
            return Err(Error::Configuration(
                "max_active must be 1..8; max_connections must be max_active+4..64",
            ));
        }
        if !(1..=128 * 1024 * 1024).contains(&self.max_request_bytes)
            || !(1..=64 * 1024 * 1024).contains(&self.max_response_bytes)
        {
            return Err(Error::Configuration(
                "request and response limits must be positive and at most 128 MiB and 64 MiB",
            ));
        }
        if !(1..=30).contains(&self.header_timeout_seconds)
            || !(1..=self.request_timeout_seconds).contains(&self.upload_timeout_seconds)
            || !(1..=1800).contains(&self.request_timeout_seconds)
            || !(1..=30).contains(&self.client_stall_seconds)
            || !(1..=600).contains(&self.startup_timeout_seconds)
            || !(self.request_timeout_seconds + self.client_stall_seconds..=1900)
                .contains(&self.shutdown_timeout_seconds)
            || !(1..=30).contains(&self.child_shutdown_seconds)
        {
            return Err(Error::Configuration(
                "timeouts are out of bounds or shutdown is shorter than request + client stall",
            ));
        }
        for path in [&self.java, &self.jar, &self.tika_config] {
            if !path.is_absolute() {
                return Err(Error::Configuration(
                    "java, jar and tika_config paths must be absolute",
                ));
            }
        }
        Ok(())
    }
}

fn decode(error: ConfigError) -> Error {
    fn details(error: &ConfigError, depth: u8) -> (String, &'static str) {
        if depth > 8 {
            return (
                String::from("<nested>"),
                "configuration nesting exceeds diagnostic bound",
            );
        }
        match error {
            ConfigError::At { key, error, .. } => {
                let (field, problem) = details(error, depth + 1);
                (key.as_deref().map_or(field, safe_field), problem)
            }
            ConfigError::Type { key, expected, .. } => (
                key.as_deref()
                    .map_or_else(|| String::from("<root>"), safe_field),
                expected,
            ),
            ConfigError::NotFound(key) => (safe_field(key), "required field missing"),
            ConfigError::FileParse { cause, .. } => (
                cause
                    .downcast_ref::<toml::de::Error>()
                    .and_then(toml::de::Error::span)
                    .map_or_else(
                        || String::from("<TOML>"),
                        |s| format!("TOML bytes {}..{}", s.start, s.end),
                    ),
                "invalid TOML syntax",
            ),
            ConfigError::Message(message) => (
                message
                    .strip_prefix("unknown field `")
                    .and_then(|s| s.split_once('`'))
                    .map_or_else(|| String::from("<root>"), |(field, _)| safe_field(field)),
                "unknown field or invalid field syntax",
            ),
            _ => (
                String::from("<source>"),
                "configuration source cannot be read or decoded",
            ),
        }
    }
    let (field, problem) = details(&error, 0);
    Error::Decoding { field, problem }
}

fn safe_field(field: &str) -> String {
    if field.len() <= 128
        && field
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'.' | b'_' | b'-'))
    {
        String::from(field)
    } else {
        String::from("<redacted>")
    }
}

pub fn debug_dotenv() -> Result<(), Error> {
    #[cfg(debug_assertions)]
    match dotenvy::from_path(".env") {
        Ok(()) => {}
        Err(dotenvy::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Configuration("debug .env could not be parsed")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn environment() -> Environment {
        let mut source = std::collections::HashMap::new();
        source.insert(
            String::from("TIKA__GATEWAY__MAX_ACTIVE"),
            String::from("secret-invalid-integer"),
        );
        Environment::with_prefix("TIKA")
            .prefix_separator("__")
            .separator("__")
            .try_parsing(true)
            .source(Some(source))
    }
    #[test]
    fn flags_override_invalid_lower_priority_types_and_errors_redact_values() {
        let settings = Settings::from_sources(
            None,
            Overrides {
                max_active: Some(1),
                ..Overrides::default()
            },
            environment(),
        )
        .unwrap();
        assert_eq!(settings.gateway.max_active, 1);
        let error = Settings::from_sources(None, Overrides::default(), environment())
            .unwrap_err()
            .to_string();
        assert!(error.contains("max_active"));
        assert!(!error.contains("secret-invalid"));
    }
    #[test]
    fn backend_is_loopback_and_deadlines_cover_accepted_work() {
        let mut settings = Gateway::default();
        assert!(settings.validate().is_ok());
        settings.backend = "10.0.0.2:9999".parse().unwrap();
        assert!(settings.validate().is_err());
        settings.backend = "127.0.0.1:9999".parse().unwrap();
        settings.ocr_ready_endpoint = Some("127.0.0.1:8081".parse().unwrap());
        assert!(settings.validate().is_ok());
        settings.ocr_ready_endpoint = Some(settings.backend);
        assert!(settings.validate().is_err());
        settings.ocr_ready_endpoint = Some("10.0.0.2:8081".parse().unwrap());
        assert!(settings.validate().is_err());
        settings.ocr_ready_endpoint = None;
        settings.shutdown_timeout_seconds = 1;
        assert!(settings.validate().is_err());
    }
}
