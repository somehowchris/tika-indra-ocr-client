//! Defaults < explicit TOML file < INDRA__SECTION__FIELD < CLI flags.

use crate::error::StartupError;
use clap::{Args, Parser, Subcommand, ValueEnum};
use config::{Config, ConfigError, Environment, File, FileFormat};
use indra_engine::{EngineConfig, MAX_BATCH, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES};
use indra_models::FetchOptions;
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, time::Duration};

pub const MAX_WORKERS: usize = 2;
pub const MAX_SLOTS: usize = 8;
pub static DEFAULT_MODEL_DIRECTORY: &str = "/models/bd02167fdf2005a19962cc15188f8496a2c87d3b";

#[derive(Parser)]
#[command(
    version,
    about = "Bounded CPU OCR for Apache Tika; serving never downloads models",
    // A bare boolean such as `--math` must not swallow a following subcommand name.
    subcommand_precedence_over_arg = true
)]
pub struct Cli {
    /// A supplied file must exist and contain valid TOML with known fields.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(flatten)]
    pub overrides: Overrides,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Verify local models, warm persistent workers, then accept OCR requests.
    Serve,
    /// Fetch or verify the pinned model cache; acquisition is never implicit.
    Models {
        #[command(subcommand)]
        action: ModelCommand,
    },
    /// Validate settings without opening model files or a listening socket.
    CheckConfig,
    /// Print the generated OpenAPI document and exit.
    Openapi,
    /// Generate shell completions (bash, zsh, fish, powershell, elvish).
    Completions { shell: clap_complete::Shell },
    /// Query a lifecycle endpoint on the configured listener; exit 1 when not OK.
    Probe { kind: ProbeKind },
    /// Remove readiness and stop admission, then finish accepted work.
    Drain,
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Download and verify the pinned revision into the cache (bounded, atomic).
    Fetch,
    /// Verify the cache offline against the locked sizes and SHA-256 values.
    Verify,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum ProbeKind {
    Ready,
    Health,
    Startup,
}

#[derive(Default, Args)]
pub struct Overrides {
    /// Loopback IP:port of the OCR HTTP listener (default 127.0.0.1:8081).
    #[arg(long, global = true)]
    pub listen: Option<SocketAddr>,
    /// Persistent native workers, 1 or 2; each owns one engine and one dedicated thread.
    #[arg(long, global = true)]
    pub workers: Option<u8>,
    /// Admitted requests waiting for a worker; workers + queue is the admission capacity.
    #[arg(long, global = true)]
    pub queue_capacity: Option<u8>,
    /// Experimental two-owner stateless batching; disabled in reference profiles.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub experimental_continuous_batching: Option<bool>,
    /// ONNX Runtime intra-op threads per session, 1 or 2; matches the CPU partition.
    #[arg(long, global = true)]
    pub threads: Option<usize>,
    /// OpenCV threads, 1 or 2.
    #[arg(long, global = true)]
    pub cv_threads: Option<usize>,
    /// Let ONNX Runtime intra-op threads spin while waiting for work.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub intra_op_spinning: Option<bool>,
    /// Let ONNX Runtime inter-op threads spin while waiting for work.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub inter_op_spinning: Option<bool>,
    /// Enable the math classifier and LaTeX recogniser (deployment default: on).
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")]
    pub math: Option<bool>,
    /// Maximum recognition rows per native call, 1-64.
    #[arg(long, global = true)]
    pub batch_size: Option<usize>,
    /// Verified model cache directory; serving never downloads.
    #[arg(long, global = true)]
    pub model_dir: Option<PathBuf>,
    /// Largest accepted encoded image upload.
    #[arg(long, global = true)]
    pub max_request_bytes: Option<usize>,
    /// Largest text response returned to a client.
    #[arg(long, global = true)]
    pub max_response_bytes: Option<usize>,
    /// Deadline for receiving the complete upload body.
    #[arg(long, global = true)]
    pub upload_timeout_seconds: Option<u64>,
    /// Absolute deadline from admission to the last response byte.
    #[arg(long, global = true)]
    pub request_timeout_seconds: Option<u64>,
    /// Grace after the request deadline before a stuck native call is terminal.
    #[arg(long, global = true)]
    pub native_timeout_grace_seconds: Option<u64>,
    /// Deadline for model verification and worker warmup before the service fails.
    #[arg(long, global = true)]
    pub startup_timeout_seconds: Option<u64>,
    /// Deadline for accepted work to finish after drain or SIGTERM.
    #[arg(long, global = true)]
    pub shutdown_timeout_seconds: Option<u64>,
    /// Connect deadline for `models fetch` (default 10).
    #[arg(long, global = true)]
    pub model_connect_timeout_seconds: Option<u64>,
    /// Per-file request deadline for `models fetch` (default 300).
    #[arg(long, global = true)]
    pub model_request_timeout_seconds: Option<u64>,
    /// JSON log targets and levels (tracing EnvFilter syntax); never gates trace export.
    #[arg(long, global = true)]
    pub log_filter: Option<String>,
    /// OTLP/HTTP base URL for protobuf traces and metrics, e.g. http://otel-collector:4318.
    #[arg(long, global = true)]
    pub otlp_endpoint: Option<String>,
    /// Authorization header value sent only to the OTLP exporter; https unless loopback.
    #[arg(long, global = true)]
    pub otlp_authorization: Option<String>,
    /// Sentry DSN for contextual failure envelopes; prefer environment or a mounted secret.
    #[arg(long, global = true)]
    pub sentry_dsn: Option<String>,
    /// service.name resource attribute (bounded identifier).
    #[arg(long, global = true)]
    pub service_name: Option<String>,
    /// deployment.environment.name resource attribute (bounded identifier).
    #[arg(long, global = true)]
    pub environment: Option<String>,
    /// Root trace sampling ratio 0.0-1.0; parent decisions are preserved across hops.
    #[arg(long, global = true)]
    pub trace_sample_ratio: Option<f64>,
    /// Bounded OTLP span queue; overflow drops spans and is reported by the SDK.
    #[arg(long, global = true)]
    pub export_queue_size: Option<usize>,
    /// Spans per OTLP export request.
    #[arg(long, global = true)]
    pub export_batch_size: Option<usize>,
    /// Deadline for one export request and for the shutdown flush.
    #[arg(long, global = true)]
    pub export_timeout_ms: Option<u64>,
    /// Scheduled delay between OTLP export batches.
    #[arg(long, global = true)]
    pub export_interval_ms: Option<u64>,
    /// Bounded asynchronous JSON log queue; a full queue drops lines and reports the count.
    #[arg(long, global = true)]
    pub log_queue_size: Option<usize>,
    /// Sentry event sampling ratio 0.0-1.0.
    #[arg(long, global = true)]
    pub sentry_sample_ratio: Option<f32>,
    /// Bounded Sentry transport queue.
    #[arg(long, global = true)]
    pub sentry_queue_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub server: ServerSettings,
    pub engine: EngineSettings,
    pub models: ModelSettings,
    pub telemetry: indra_telemetry::Options,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub listen: SocketAddr,
    pub workers: u8,
    pub queue_capacity: u8,
    pub experimental_continuous_batching: bool,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub upload_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub native_timeout_grace_seconds: u64,
    pub startup_timeout_seconds: u64,
    pub shutdown_timeout_seconds: u64,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8081)),
            workers: 1,
            queue_capacity: 2,
            experimental_continuous_batching: false,
            max_request_bytes: MAX_INPUT_BYTES,
            max_response_bytes: MAX_OUTPUT_BYTES,
            upload_timeout_seconds: 10,
            request_timeout_seconds: 60,
            native_timeout_grace_seconds: 5,
            startup_timeout_seconds: 120,
            shutdown_timeout_seconds: 75,
        }
    }
}

impl ServerSettings {
    pub const fn admission_capacity(&self) -> u16 {
        self.workers as u16 + self.queue_capacity as u16
    }
    pub const fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_seconds)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineSettings {
    pub threads: usize,
    /// Unset inherits ORT threads; explicitly set one for the controlled baseline.
    pub cv_threads: Option<usize>,
    pub intra_op_spinning: bool,
    pub inter_op_spinning: bool,
    pub math: bool,
    pub batch_size: usize,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            threads: 1,
            cv_threads: None,
            intra_op_spinning: true,
            inter_op_spinning: true,
            math: false,
            batch_size: MAX_BATCH,
        }
    }
}

impl EngineSettings {
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            threads: self.threads,
            cv_threads: self.cv_threads.unwrap_or(self.threads),
            intra_op_spinning: self.intra_op_spinning,
            inter_op_spinning: self.inter_op_spinning,
            math: self.math,
            batch_size: self.batch_size,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelSettings {
    pub directory: PathBuf,
    pub connect_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            directory: PathBuf::from(DEFAULT_MODEL_DIRECTORY),
            connect_timeout_seconds: 10,
            request_timeout_seconds: 300,
        }
    }
}

impl ModelSettings {
    pub const fn fetch_options(&self) -> FetchOptions {
        FetchOptions {
            connect_timeout: Duration::from_secs(self.connect_timeout_seconds),
            request_timeout: Duration::from_secs(self.request_timeout_seconds),
        }
    }
}

impl Settings {
    pub fn load(file: Option<PathBuf>, flags: Overrides) -> Result<Self, StartupError> {
        Self::from_sources(file, flags, environment())
    }

    fn from_sources(
        file: Option<PathBuf>,
        flags: Overrides,
        environment: Environment,
    ) -> Result<Self, StartupError> {
        let mut builder = Config::builder();
        if let Some(file) = file {
            builder = builder.add_source(File::from(file).format(FileFormat::Toml).required(true));
        }
        // Resolve all source priorities before typed decoding: a valid flag can
        // override an invalid lower-priority value as well as repair a profile.
        let merged = flags
            .merge(builder.add_source(environment))?
            .build()
            .map_err(|error| config_error("read/merge", error))?;
        let settings: Self = merged
            .try_deserialize()
            .map_err(|error| config_error("typed decoding", error))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), StartupError> {
        validate_runtime(&self.server, &self.engine.engine_config())?;
        if self.models.directory.as_os_str().is_empty()
            || self.models.connect_timeout_seconds == 0
            || self.models.connect_timeout_seconds > self.models.request_timeout_seconds
            || self.models.request_timeout_seconds > 1800
        {
            return Err(StartupError::InvalidConfig(
                "models need a directory and 0 < connect timeout ≤ request timeout ≤1800s",
            ));
        }
        self.telemetry.validate()?;
        Ok(())
    }
}

impl Overrides {
    fn merge(
        self,
        mut builder: config::ConfigBuilder<config::builder::DefaultState>,
    ) -> Result<config::ConfigBuilder<config::builder::DefaultState>, StartupError> {
        macro_rules! assign {
            ($key:literal, $value:expr) => {
                if let Some(value) = $value {
                    builder = builder
                        .set_override($key, value)
                        .map_err(|error| config_error("CLI override", error))?;
                }
            };
        }
        assign!("server.listen", self.listen.map(|value| value.to_string()));
        assign!("server.workers", self.workers);
        assign!("server.queue_capacity", self.queue_capacity);
        assign!(
            "server.experimental_continuous_batching",
            self.experimental_continuous_batching
        );
        assign!("engine.threads", self.threads.map(|value| value as u64));
        assign!(
            "engine.cv_threads",
            self.cv_threads.map(|value| value as u64)
        );
        assign!("engine.intra_op_spinning", self.intra_op_spinning);
        assign!("engine.inter_op_spinning", self.inter_op_spinning);
        assign!("engine.math", self.math);
        assign!(
            "engine.batch_size",
            self.batch_size.map(|value| value as u64)
        );
        if let Some(directory) = self.model_dir {
            let directory = directory
                .into_os_string()
                .into_string()
                .map_err(|_| StartupError::InvalidConfig("models.directory must be valid UTF-8"))?;
            assign!("models.directory", Some(directory));
        }
        assign!(
            "server.max_request_bytes",
            self.max_request_bytes.map(|value| value as u64)
        );
        assign!(
            "server.max_response_bytes",
            self.max_response_bytes.map(|value| value as u64)
        );
        assign!("server.upload_timeout_seconds", self.upload_timeout_seconds);
        assign!(
            "server.request_timeout_seconds",
            self.request_timeout_seconds
        );
        assign!(
            "server.native_timeout_grace_seconds",
            self.native_timeout_grace_seconds
        );
        assign!(
            "server.startup_timeout_seconds",
            self.startup_timeout_seconds
        );
        assign!(
            "server.shutdown_timeout_seconds",
            self.shutdown_timeout_seconds
        );
        assign!(
            "models.connect_timeout_seconds",
            self.model_connect_timeout_seconds
        );
        assign!(
            "models.request_timeout_seconds",
            self.model_request_timeout_seconds
        );
        assign!("telemetry.log_filter", self.log_filter);
        assign!("telemetry.otlp_endpoint", self.otlp_endpoint);
        assign!("telemetry.otlp_authorization", self.otlp_authorization);
        assign!("telemetry.sentry_dsn", self.sentry_dsn);
        assign!("telemetry.service_name", self.service_name);
        assign!("telemetry.environment", self.environment);
        assign!("telemetry.trace_sample_ratio", self.trace_sample_ratio);
        assign!(
            "telemetry.export_queue_size",
            self.export_queue_size.map(|value| value as u64)
        );
        assign!(
            "telemetry.export_batch_size",
            self.export_batch_size.map(|value| value as u64)
        );
        assign!("telemetry.export_timeout_ms", self.export_timeout_ms);
        assign!("telemetry.export_interval_ms", self.export_interval_ms);
        assign!(
            "telemetry.log_queue_size",
            self.log_queue_size.map(|value| value as u64)
        );
        assign!(
            "telemetry.sentry_sample_ratio",
            self.sentry_sample_ratio.map(f64::from)
        );
        assign!(
            "telemetry.sentry_queue_size",
            self.sentry_queue_size.map(|value| value as u64)
        );
        Ok(builder)
    }
}

pub(crate) fn validate_runtime(s: &ServerSettings, e: &EngineConfig) -> Result<(), StartupError> {
    if s.listen.port() == 0 {
        return Err(StartupError::InvalidConfig(
            "server.listen must use a nonzero port",
        ));
    }
    if !(1..=MAX_WORKERS).contains(&usize::from(s.workers)) {
        return Err(StartupError::InvalidConfig("server.workers must be 1 or 2"));
    }
    if s.queue_capacity == 0 || usize::from(s.admission_capacity()) > MAX_SLOTS {
        return Err(StartupError::InvalidConfig(
            "queue_capacity must be positive; workers + queue_capacity cannot exceed 8",
        ));
    }
    if s.experimental_continuous_batching && s.workers != 1 {
        return Err(StartupError::InvalidConfig(
            "experimental continuous batching uses exactly one native worker and two already-admitted owners",
        ));
    }
    if !(1..=2).contains(&e.threads) || !(1..=2).contains(&e.cv_threads) {
        return Err(StartupError::InvalidConfig(
            "engine threads and CV threads must be 1 or 2",
        ));
    }
    if usize::from(s.workers) * e.threads.max(e.cv_threads) > 2 {
        return Err(StartupError::InvalidConfig(
            "workers × max(threads, cv_threads) cannot exceed the two-CPU profile",
        ));
    }
    if !(1..=MAX_BATCH).contains(&e.batch_size) {
        return Err(StartupError::InvalidConfig(
            "engine.batch_size must be 1..=64",
        ));
    }
    if !(1..=MAX_INPUT_BYTES).contains(&s.max_request_bytes)
        || !(1..=MAX_OUTPUT_BYTES).contains(&s.max_response_bytes)
    {
        return Err(StartupError::InvalidConfig(
            "request/response byte limits must be positive and cannot exceed 20 MiB / 4 MiB",
        ));
    }
    if !(1..=300).contains(&s.request_timeout_seconds)
        || s.upload_timeout_seconds == 0
        || s.upload_timeout_seconds > s.request_timeout_seconds
        || !(1..=10).contains(&s.native_timeout_grace_seconds)
        || !(1..=600).contains(&s.startup_timeout_seconds)
        || s.shutdown_timeout_seconds < s.request_timeout_seconds + s.native_timeout_grace_seconds
        || s.shutdown_timeout_seconds > 600
    {
        return Err(StartupError::InvalidConfig(
            "invalid timeout ordering: upload ≤ request(1..300s); native grace 1..10s; shutdown ≥ request + grace and ≤600s; startup 1..600s",
        ));
    }
    Ok(())
}

fn environment() -> Environment {
    Environment::with_prefix("INDRA")
        .prefix_separator("__")
        .separator("__")
        .try_parsing(true)
}

/// Preserve actionable key/type or parser byte-range diagnostics without
/// forwarding config-rs Display: it includes the actual unexpected value and
/// TOML source snippets, which can contain telemetry credentials.
fn config_error(stage: &'static str, error: ConfigError) -> StartupError {
    let (field, problem) = safe_config_error(&error, 0);
    StartupError::Configuration {
        stage,
        field,
        problem,
    }
}

fn safe_config_error(error: &ConfigError, depth: u8) -> (String, &'static str) {
    if depth >= 8 {
        return (
            String::from("<nested field>"),
            "configuration nesting exceeds diagnostic bound",
        );
    }
    match error {
        ConfigError::Type { key, expected, .. } => {
            (safe_field(key.as_deref().unwrap_or("<root>")), expected)
        }
        ConfigError::NotFound(key) => (safe_field(key), "required configuration field is missing"),
        ConfigError::At { error, key, .. } => {
            let (leaf, problem) = safe_config_error(error, depth + 1);
            let field = match key.as_deref() {
                Some(key) => {
                    let path = safe_field(key);
                    if !leaf.starts_with('<')
                        && !path.ends_with(&leaf)
                        && path.len() + leaf.len() < 128
                    {
                        format!("{path}.{leaf}")
                    } else {
                        path
                    }
                }
                None => leaf,
            };
            (field, problem)
        }
        ConfigError::Message(message) => {
            let unknown = message
                .strip_prefix("unknown field `")
                .and_then(|rest| rest.split_once('`'))
                .map(|(field, _)| field);
            (
                safe_field(unknown.unwrap_or("<root>")),
                if unknown.is_some() {
                    "unknown field; remove it or use a documented field name"
                } else {
                    "value does not satisfy the field's declared type or syntax"
                },
            )
        }
        ConfigError::FileParse { cause, .. } => {
            let location = cause
                .downcast_ref::<toml::de::Error>()
                .and_then(toml::de::Error::span)
                .map_or_else(
                    || String::from("<TOML file>"),
                    |span| format!("TOML bytes {}..{}", span.start, span.end),
                );
            (
                location,
                "invalid TOML syntax; byte positions are zero-based and source values are redacted",
            )
        }
        ConfigError::Foreign(cause) => {
            let problem = cause.downcast_ref::<std::io::Error>().map_or(
                "configuration source could not be read",
                |error| match error.kind() {
                    std::io::ErrorKind::NotFound => "explicit configuration file does not exist",
                    std::io::ErrorKind::PermissionDenied => {
                        "permission denied reading configuration file"
                    }
                    _ => "I/O failure reading configuration file",
                },
            );
            (String::from("<configuration source>"), problem)
        }
        _ => (
            String::from("<configuration source>"),
            "invalid configuration source or key path",
        ),
    }
}

fn safe_field(field: &str) -> String {
    if field.len() <= 128
        && field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        String::from(field)
    } else {
        String::from("<redacted field>")
    }
}

/// Called before a runtime or worker exists. Release never reads .env.
pub fn debug_dotenv() -> Result<(), StartupError> {
    #[cfg(debug_assertions)]
    match dotenvy::from_path(".env") {
        Ok(()) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(StartupError::Dotenv),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn isolated(entries: &[(&str, &str)]) -> Environment {
        let mut source = HashMap::new();
        for (key, value) in entries {
            source.insert(String::from(*key), String::from(*value));
        }
        environment().source(Some(source))
    }

    #[test]
    fn defaults_validate_and_cv_inherits_ort_threads() {
        let settings =
            Settings::from_sources(None, Overrides::default(), isolated(&[])).expect("defaults");
        assert_eq!(settings.server.admission_capacity(), 3);
        assert!(!settings.server.experimental_continuous_batching);
        assert_eq!(
            settings.models.directory,
            PathBuf::from(DEFAULT_MODEL_DIRECTORY)
        );
        let settings = Settings::from_sources(
            None,
            Overrides {
                threads: Some(2),
                ..Overrides::default()
            },
            isolated(&[]),
        )
        .expect("inherited CV threads");
        assert_eq!(settings.engine.engine_config().cv_threads, 2);
    }

    #[test]
    fn continuous_pilot_is_opt_in_and_retains_the_existing_admission_bound() {
        let settings = Settings::from_sources(
            None,
            Overrides::default(),
            isolated(&[("INDRA__SERVER__EXPERIMENTAL_CONTINUOUS_BATCHING", "true")]),
        )
        .expect("explicit experimental environment setting");
        assert!(settings.server.experimental_continuous_batching);
        assert_eq!(settings.server.admission_capacity(), 3);
        let overridden = Settings::from_sources(
            None,
            Overrides {
                experimental_continuous_batching: Some(false),
                ..Overrides::default()
            },
            isolated(&[("INDRA__SERVER__EXPERIMENTAL_CONTINUOUS_BATCHING", "true")]),
        )
        .expect("flag disables a lower-priority experiment");
        assert!(!overridden.server.experimental_continuous_batching);
        assert!(
            Settings::from_sources(
                None,
                Overrides {
                    experimental_continuous_batching: Some(true),
                    workers: Some(2),
                    ..Overrides::default()
                },
                isolated(&[])
            )
            .is_err(),
            "the pilot has one shared inference owner"
        );
    }

    #[test]
    fn flags_override_environment_before_validation() {
        let settings = Settings::from_sources(
            None,
            Overrides {
                workers: Some(1),
                math: Some(false),
                ..Overrides::default()
            },
            isolated(&[
                ("INDRA__SERVER__WORKERS", "2"),
                ("INDRA__ENGINE__THREADS", "2"),
                ("INDRA__ENGINE__MATH", "true"),
            ]),
        )
        .expect("flag repairs worker/thread combination");
        assert_eq!(settings.server.workers, 1);
        assert!(!settings.engine.math);
        let settings = Settings::from_sources(
            None,
            Overrides {
                threads: Some(1),
                ..Overrides::default()
            },
            isolated(&[("INDRA__ENGINE__THREADS", "invalid-lower-priority-value")]),
        )
        .expect("flag overrides before typed decoding");
        assert_eq!(settings.engine.threads, 1);
    }

    #[test]
    fn unknown_fields_and_invalid_cross_field_limits_fail() {
        assert!(
            Settings::from_sources(
                None,
                Overrides::default(),
                isolated(&[("INDRA__SERVER__TYPO", "1")])
            )
            .is_err()
        );
        for flags in [
            Overrides {
                workers: Some(2),
                threads: Some(2),
                ..Overrides::default()
            },
            Overrides {
                max_request_bytes: Some(0),
                ..Overrides::default()
            },
            Overrides {
                shutdown_timeout_seconds: Some(60),
                ..Overrides::default()
            },
        ] {
            assert!(Settings::from_sources(None, flags, isolated(&[])).is_err());
        }
        assert!(
            Settings::from_sources(
                None,
                Overrides::default(),
                isolated(&[("INDRA__TELEMETRY__SEND_PII", "true")])
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_missing_config_is_an_error() {
        assert!(
            Settings::from_sources(
                Some(PathBuf::from("/missing-indra-config.toml")),
                Overrides::default(),
                isolated(&[])
            )
            .is_err()
        );
    }

    #[test]
    fn file_environment_and_flags_resolve_in_documented_order() {
        let file = std::env::temp_dir().join(format!(
            "indra-config-precedence-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &file,
            "[server]\nqueue_capacity = 1\n[engine]\nthreads = 2\n",
        )
        .expect("isolated profile file");
        let from_env = Settings::from_sources(
            Some(PathBuf::from(file.as_path())),
            Overrides::default(),
            isolated(&[("INDRA__ENGINE__THREADS", "1")]),
        )
        .expect("environment overrides file");
        let from_flag = Settings::from_sources(
            Some(PathBuf::from(file.as_path())),
            Overrides {
                threads: Some(2),
                ..Overrides::default()
            },
            isolated(&[("INDRA__ENGINE__THREADS", "1")]),
        )
        .expect("flag overrides environment");
        std::fs::remove_file(file).expect("remove isolated profile");
        assert_eq!(from_env.server.queue_capacity, 1);
        assert_eq!(from_env.engine.threads, 1);
        assert_eq!(from_flag.engine.threads, 2);
    }

    #[test]
    fn shipped_example_profile_loads_and_matches_the_deployed_math_setting() {
        // docs/runtime-service.md tells operators to copy this file; a renamed
        // or removed field must fail here rather than in their deployment.
        let file =
            std::env::temp_dir().join(format!("indra-config-example-{}.toml", std::process::id()));
        std::fs::write(&file, include_str!("../../../indra.example.toml"))
            .expect("isolated copy of the shipped example");
        let loaded = Settings::from_sources(
            Some(PathBuf::from(file.as_path())),
            Overrides::default(),
            isolated(&[]),
        );
        std::fs::remove_file(file).expect("remove isolated example");
        let settings = loaded.expect("shipped example validates");
        assert!(
            settings.engine.math,
            "Compose and Kubernetes serve with math"
        );
        assert_eq!(settings.engine.cv_threads, Some(1));
        assert_eq!(
            settings.models.directory,
            PathBuf::from(DEFAULT_MODEL_DIRECTORY)
        );
    }

    #[test]
    fn type_diagnostics_name_fields_without_exposing_values() {
        let failure = Settings::from_sources(
            None,
            Overrides::default(),
            isolated(&[("INDRA__ENGINE__THREADS", "a-private-secret")]),
        )
        .expect_err("invalid integer");
        let message = failure.to_string();
        assert!(
            message.contains("engine.threads"),
            "actionable path: {message}"
        );
        assert!(!message.contains("a-private-secret"));
        let invalid = ConfigError::FileParse {
            uri: None,
            cause: "untrusted source text".into(),
        };
        assert!(
            !config_error("read", invalid)
                .to_string()
                .contains("untrusted source text")
        );
    }
}
