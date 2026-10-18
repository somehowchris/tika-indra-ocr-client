//! Defaults < explicit TOML file < INDRA__SECTION__FIELD < CLI flags.

use std::{net::SocketAddr, path::PathBuf, time::Duration};
use clap::{Args, Parser, Subcommand, ValueEnum};
use config::{Config, Environment, File, FileFormat};
use indra_engine::{EngineConfig, MAX_BATCH, MAX_INPUT_BYTES, MAX_TEXT_BYTES};
use indra_models::FetchOptions;
use serde::Deserialize;
use crate::error::StartupError;

pub const MAX_WORKERS: usize = 2;
pub const MAX_SLOTS: usize = 8;
pub static DEFAULT_MODEL_DIRECTORY: &str = "/models/ed345c7f158eddbc7dee0c6a7269afb75a94f7e8";

#[derive(Parser)]
#[command(version, about = "Bounded CPU OCR for Apache Tika; serving never downloads models")]
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
    Models { #[command(subcommand)] action: ModelCommand },
    /// Validate settings without opening model files or a listening socket.
    CheckConfig,
    Openapi,
    Completions { shell: clap_complete::Shell },
    Probe { kind: ProbeKind },
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand { Fetch, Verify }

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum ProbeKind { Ready, Health }

#[derive(Default, Args)]
pub struct Overrides {
    #[arg(long, global = true)] pub listen: Option<SocketAddr>,
    #[arg(long, global = true)] pub workers: Option<u8>,
    #[arg(long, global = true)] pub queue_capacity: Option<u8>,
    #[arg(long, global = true)] pub threads: Option<usize>,
    #[arg(long, global = true)] pub cv_threads: Option<usize>,
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")] pub intra_op_spinning: Option<bool>,
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")] pub inter_op_spinning: Option<bool>,
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true")] pub math: Option<bool>,
    #[arg(long, global = true)] pub batch_size: Option<usize>,
    #[arg(long, global = true)] pub model_dir: Option<PathBuf>,
    #[arg(long, global = true)] pub max_request_bytes: Option<usize>,
    #[arg(long, global = true)] pub max_response_bytes: Option<usize>,
    #[arg(long, global = true)] pub upload_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub request_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub native_timeout_grace_seconds: Option<u64>,
    #[arg(long, global = true)] pub startup_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub shutdown_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub model_connect_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub model_request_timeout_seconds: Option<u64>,
    #[arg(long, global = true)] pub log_filter: Option<String>,
    #[arg(long, global = true)] pub otlp_endpoint: Option<String>,
    #[arg(long, global = true)] pub otlp_authorization: Option<String>,
    #[arg(long, global = true)] pub sentry_dsn: Option<String>,
    #[arg(long, global = true)] pub service_name: Option<String>,
    #[arg(long, global = true)] pub environment: Option<String>,
    #[arg(long, global = true)] pub trace_sample_ratio: Option<f64>,
    #[arg(long, global = true)] pub export_queue_size: Option<usize>,
    #[arg(long, global = true)] pub export_batch_size: Option<usize>,
    #[arg(long, global = true)] pub export_timeout_ms: Option<u64>,
    #[arg(long, global = true)] pub export_interval_ms: Option<u64>,
    #[arg(long, global = true)] pub log_queue_size: Option<usize>,
    #[arg(long, global = true)] pub sentry_sample_ratio: Option<f32>,
    #[arg(long, global = true)] pub sentry_queue_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub server: ServerSettings,
    pub engine: EngineSettings,
    pub models: ModelSettings,
    pub telemetry: indra_telemetry::Options,
}

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub listen: SocketAddr,
    pub workers: u8,
    pub queue_capacity: u8,
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
            listen: SocketAddr::from(([127, 0, 0, 1], 8081)), workers: 1, queue_capacity: 2,
            max_request_bytes: MAX_INPUT_BYTES, max_response_bytes: MAX_TEXT_BYTES,
            upload_timeout_seconds: 10, request_timeout_seconds: 60,
            native_timeout_grace_seconds: 5, startup_timeout_seconds: 120,
            shutdown_timeout_seconds: 75,
        }
    }
}

impl ServerSettings {
    pub const fn admission_capacity(&self) -> u16 { self.workers as u16 + self.queue_capacity as u16 }
    pub const fn request_timeout(&self) -> Duration { Duration::from_secs(self.request_timeout_seconds) }
}

#[derive(Debug, Copy, Clone, Deserialize)]
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
        Self { threads: 1, cv_threads: None, intra_op_spinning: true, inter_op_spinning: true, math: false, batch_size: MAX_BATCH }
    }
}

impl EngineSettings {
    pub fn engine_config(&self) -> EngineConfig {
        EngineConfig {
            threads: self.threads, cv_threads: self.cv_threads.unwrap_or(self.threads),
            intra_op_spinning: self.intra_op_spinning, inter_op_spinning: self.inter_op_spinning,
            math: self.math, batch_size: self.batch_size,
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
        Self { directory: PathBuf::from(DEFAULT_MODEL_DIRECTORY), connect_timeout_seconds: 10, request_timeout_seconds: 300 }
    }
}

impl ModelSettings {
    pub fn fetch_options(&self) -> FetchOptions {
        FetchOptions { connect_timeout: Duration::from_secs(self.connect_timeout_seconds), request_timeout: Duration::from_secs(self.request_timeout_seconds) }
    }
}

impl Settings {
    pub fn load(file: Option<PathBuf>, flags: Overrides) -> Result<Self, StartupError> {
        Self::from_sources(file, flags, environment())
    }

    fn from_sources(file: Option<PathBuf>, flags: Overrides, environment: Environment) -> Result<Self, StartupError> {
        let mut builder = Config::builder();
        if let Some(file) = file {
            builder = builder.add_source(File::from(file).format(FileFormat::Toml).required(true));
        }
        let merged = builder.add_source(environment).build().map_err(|_| StartupError::Configuration("read/merge"))?;
        // Deserialize defaults apply only after file/environment merging. CLI overrides
        // must be applied before cross-field validation so a flag can repair a profile.
        let mut settings: Self = merged.try_deserialize().map_err(|_| StartupError::Configuration("typed decoding"))?;
        settings.apply(flags);
        settings.validate()?;
        Ok(settings)
    }

    fn apply(&mut self, flags: Overrides) {
        macro_rules! assign { ($destination:expr, $value:expr) => { if let Some(value) = $value { $destination = value; } }; }
        assign!(self.server.listen, flags.listen);
        assign!(self.server.workers, flags.workers);
        assign!(self.server.queue_capacity, flags.queue_capacity);
        assign!(self.engine.threads, flags.threads);
        if let Some(value) = flags.cv_threads { self.engine.cv_threads = Some(value); }
        assign!(self.engine.intra_op_spinning, flags.intra_op_spinning);
        assign!(self.engine.inter_op_spinning, flags.inter_op_spinning);
        assign!(self.engine.math, flags.math);
        assign!(self.engine.batch_size, flags.batch_size);
        assign!(self.models.directory, flags.model_dir);
        assign!(self.server.max_request_bytes, flags.max_request_bytes);
        assign!(self.server.max_response_bytes, flags.max_response_bytes);
        assign!(self.server.upload_timeout_seconds, flags.upload_timeout_seconds);
        assign!(self.server.request_timeout_seconds, flags.request_timeout_seconds);
        assign!(self.server.native_timeout_grace_seconds, flags.native_timeout_grace_seconds);
        assign!(self.server.startup_timeout_seconds, flags.startup_timeout_seconds);
        assign!(self.server.shutdown_timeout_seconds, flags.shutdown_timeout_seconds);
        assign!(self.models.connect_timeout_seconds, flags.model_connect_timeout_seconds);
        assign!(self.models.request_timeout_seconds, flags.model_request_timeout_seconds);
        assign!(self.telemetry.log_filter, flags.log_filter);
        if let Some(value) = flags.otlp_endpoint { self.telemetry.otlp_endpoint = Some(value); }
        if let Some(value) = flags.otlp_authorization { self.telemetry.otlp_authorization = Some(value); }
        if let Some(value) = flags.sentry_dsn { self.telemetry.sentry_dsn = Some(value); }
        assign!(self.telemetry.service_name, flags.service_name);
        assign!(self.telemetry.environment, flags.environment);
        assign!(self.telemetry.trace_sample_ratio, flags.trace_sample_ratio);
        assign!(self.telemetry.export_queue_size, flags.export_queue_size);
        assign!(self.telemetry.export_batch_size, flags.export_batch_size);
        assign!(self.telemetry.export_timeout_ms, flags.export_timeout_ms);
        assign!(self.telemetry.export_interval_ms, flags.export_interval_ms);
        assign!(self.telemetry.log_queue_size, flags.log_queue_size);
        assign!(self.telemetry.sentry_sample_ratio, flags.sentry_sample_ratio);
        assign!(self.telemetry.sentry_queue_size, flags.sentry_queue_size);
    }

    pub fn validate(&self) -> Result<(), StartupError> {
        validate_runtime(&self.server, &self.engine.engine_config())?;
        if self.models.directory.as_os_str().is_empty()
            || self.models.connect_timeout_seconds == 0
            || self.models.connect_timeout_seconds > self.models.request_timeout_seconds
            || self.models.request_timeout_seconds > 1800
        {
            return Err(StartupError::InvalidConfig("models need a directory and 0 < connect timeout ≤ request timeout ≤1800s"));
        }
        self.telemetry.validate()?;
        Ok(())
    }
}

pub(crate) fn validate_runtime(s: &ServerSettings, e: &EngineConfig) -> Result<(), StartupError> {
    if s.listen.port() == 0 { return Err(StartupError::InvalidConfig("server.listen must use a nonzero port")); }
    if !(1..=MAX_WORKERS).contains(&usize::from(s.workers)) { return Err(StartupError::InvalidConfig("server.workers must be 1 or 2")); }
    if s.queue_capacity == 0 || usize::from(s.admission_capacity()) > MAX_SLOTS {
        return Err(StartupError::InvalidConfig("queue_capacity must be positive; workers + queue_capacity cannot exceed 8"));
    }
    if !(1..=2).contains(&e.threads) || !(1..=2).contains(&e.cv_threads) {
        return Err(StartupError::InvalidConfig("engine threads and CV threads must be 1 or 2"));
    }
    if usize::from(s.workers) * e.threads.max(e.cv_threads) > 2 {
        return Err(StartupError::InvalidConfig("workers × max(threads, cv_threads) cannot exceed the two-CPU profile"));
    }
    if !(1..=MAX_BATCH).contains(&e.batch_size) { return Err(StartupError::InvalidConfig("engine.batch_size must be 1..=64")); }
    if !(1..=MAX_INPUT_BYTES).contains(&s.max_request_bytes) || !(1..=MAX_TEXT_BYTES).contains(&s.max_response_bytes) {
        return Err(StartupError::InvalidConfig("request/response byte limits must be positive and cannot exceed 20 MiB / 4 MiB"));
    }
    if !(1..=300).contains(&s.request_timeout_seconds)
        || s.upload_timeout_seconds == 0 || s.upload_timeout_seconds > s.request_timeout_seconds
        || !(1..=10).contains(&s.native_timeout_grace_seconds)
        || !(1..=600).contains(&s.startup_timeout_seconds)
        || s.shutdown_timeout_seconds < s.request_timeout_seconds + s.native_timeout_grace_seconds
        || s.shutdown_timeout_seconds > 600
    {
        return Err(StartupError::InvalidConfig("invalid timeout ordering: upload ≤ request(1..300s); native grace 1..10s; shutdown ≥ request + grace and ≤600s; startup 1..600s"));
    }
    Ok(())
}

fn environment() -> Environment {
    Environment::with_prefix("INDRA").prefix_separator("__").separator("__").try_parsing(true)
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
        for (key, value) in entries { source.insert(String::from(*key), String::from(*value)); }
        environment().source(Some(source))
    }

    #[test]
    fn defaults_validate_and_cv_inherits_ort_threads() {
        let settings = Settings::from_sources(None, Overrides::default(), isolated(&[])).expect("defaults");
        assert_eq!(settings.server.admission_capacity(), 3);
        assert_eq!(settings.models.directory, PathBuf::from(DEFAULT_MODEL_DIRECTORY));
        let settings = Settings::from_sources(None, Overrides { threads: Some(2), ..Overrides::default() }, isolated(&[])).expect("inherited CV threads");
        assert_eq!(settings.engine.engine_config().cv_threads, 2);
    }

    #[test]
    fn flags_override_environment_before_validation() {
        let settings = Settings::from_sources(None, Overrides { workers: Some(1), math: Some(false), ..Overrides::default() }, isolated(&[("INDRA__SERVER__WORKERS", "2"), ("INDRA__ENGINE__THREADS", "2"), ("INDRA__ENGINE__MATH", "true")])).expect("flag repairs worker/thread combination");
        assert_eq!(settings.server.workers, 1);
        assert!(!settings.engine.math);
    }

    #[test]
    fn unknown_fields_and_invalid_cross_field_limits_fail() {
        assert!(Settings::from_sources(None, Overrides::default(), isolated(&[("INDRA__SERVER__TYPO", "1")])).is_err());
        for flags in [Overrides { workers: Some(2), threads: Some(2), ..Overrides::default() }, Overrides { max_request_bytes: Some(0), ..Overrides::default() }, Overrides { shutdown_timeout_seconds: Some(60), ..Overrides::default() }] {
            assert!(Settings::from_sources(None, flags, isolated(&[])).is_err());
        }
        assert!(Settings::from_sources(None, Overrides::default(), isolated(&[("INDRA__TELEMETRY__SEND_PII", "true")])).is_err());
    }

    #[test]
    fn explicit_missing_config_is_an_error() {
        assert!(Settings::from_sources(Some(PathBuf::from("/missing-indra-config.toml")), Overrides::default(), isolated(&[])).is_err());
    }
}
