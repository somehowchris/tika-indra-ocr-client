use crate::TelemetryError;
use serde::Deserialize;
use std::fmt;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub log_filter: String,
    pub service_name: String,
    pub environment: String,
    /// Base OTLP/HTTP URL. The service appends /v1/traces and /v1/metrics.
    pub otlp_endpoint: Option<String>,
    pub otlp_authorization: Option<String>,
    pub trace_sample_ratio: f64,
    pub export_queue_size: usize,
    pub export_batch_size: usize,
    pub export_timeout_ms: u64,
    pub export_interval_ms: u64,
    pub log_queue_size: usize,
    pub sentry_dsn: Option<String>,
    pub sentry_sample_ratio: f32,
    pub sentry_queue_size: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            log_filter: String::from("info"),
            service_name: String::from("indra-ocr"),
            environment: String::from("local"),
            otlp_endpoint: None,
            otlp_authorization: None,
            trace_sample_ratio: 0.1,
            export_queue_size: 512,
            export_batch_size: 64,
            export_timeout_ms: 2000,
            export_interval_ms: 5000,
            log_queue_size: 512,
            sentry_dsn: None,
            sentry_sample_ratio: 1.0,
            sentry_queue_size: 16,
        }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Options")
            .field("otlp_enabled", &self.otlp_endpoint.is_some())
            .field("sentry_enabled", &self.sentry_dsn.is_some())
            .field("export_queue_size", &self.export_queue_size)
            .finish_non_exhaustive()
    }
}

impl Options {
    pub fn validate(&self) -> Result<(), TelemetryError> {
        if self.log_filter.is_empty()
            || self.log_filter.len() > 512
            || !identifier(&self.service_name)
            || !identifier(&self.environment)
        {
            return Err(TelemetryError::Configuration(
                "bounded log_filter/service_name/environment",
            ));
        }
        tracing_subscriber::EnvFilter::try_new(&self.log_filter)
            .map_err(|_| TelemetryError::Configuration("log_filter syntax"))?;
        if !(0.0..=1.0).contains(&self.trace_sample_ratio)
            || !(0.0..=1.0).contains(&self.sentry_sample_ratio)
        {
            return Err(TelemetryError::Configuration(
                "sampling ratios must be finite and between zero and one",
            ));
        }
        if !(1..=4096).contains(&self.export_queue_size)
            || !(1..=self.export_queue_size).contains(&self.export_batch_size)
            || !(100..=5000).contains(&self.export_timeout_ms)
            || !(100..=60_000).contains(&self.export_interval_ms)
            || !(1..=4096).contains(&self.log_queue_size)
            || !(1..=256).contains(&self.sentry_queue_size)
        {
            return Err(TelemetryError::Configuration(
                "bounded queue sizes/export timeouts",
            ));
        }
        #[cfg(not(feature = "otlp"))]
        if self.otlp_endpoint.is_some() {
            return Err(TelemetryError::Feature("otlp"));
        }
        #[cfg(feature = "otlp")]
        if let Some(endpoint) = &self.otlp_endpoint {
            let parsed = url::Url::parse(endpoint)
                .map_err(|_| TelemetryError::Configuration("otlp_endpoint URL"))?;
            if endpoint.len() > 2048
                || !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
            {
                return Err(TelemetryError::Configuration(
                    "otlp_endpoint must be an HTTP(S) base URL without userinfo/query/fragment",
                ));
            }
        }
        if let Some(authorization) = &self.otlp_authorization
            && (self.otlp_endpoint.is_none()
                || authorization.len() > 4096
                || authorization.is_empty()
                || authorization.bytes().any(|byte| !matches!(byte, 32..=126)))
        {
            return Err(TelemetryError::Configuration(
                "otlp_authorization requires an endpoint and a bounded ASCII header value",
            ));
        }
        #[cfg(feature = "otlp")]
        if let (Some(endpoint), Some(_)) = (&self.otlp_endpoint, &self.otlp_authorization)
            && let Ok(parsed) = url::Url::parse(endpoint)
            && parsed.scheme() == "http"
            && !loopback(parsed.host())
        {
            // A credential must not travel in clear text to a remote collector;
            // an in-pod or node-local agent on loopback stays allowed.
            return Err(TelemetryError::Configuration(
                "otlp_authorization over plaintext http is allowed only to a loopback collector",
            ));
        }
        #[cfg(not(feature = "sentry"))]
        if self.sentry_dsn.is_some() {
            return Err(TelemetryError::Feature("sentry"));
        }
        #[cfg(feature = "sentry")]
        if let Some(dsn) = &self.sentry_dsn
            && (dsn.len() > 2048 || dsn.parse::<sentry::types::Dsn>().is_err())
        {
            return Err(TelemetryError::Configuration("sentry_dsn"));
        }
        Ok(())
    }
}

#[cfg(feature = "otlp")]
fn loopback(host: Option<url::Host<&str>>) -> bool {
    match host {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_bounded_and_debug_redacts_credentials() {
        let options = Options::default();
        options.validate().expect("bounded defaults");
        let private = Options {
            sentry_dsn: Some(String::from("private-secret")),
            ..Options::default()
        };
        assert!(!format!("{private:?}").contains("private-secret"));
    }

    #[test]
    fn ratios_cardinality_and_queue_limits_are_enforced() {
        for invalid in [
            Options {
                trace_sample_ratio: f64::NAN,
                ..Options::default()
            },
            Options {
                environment: String::from("unbounded/user/data"),
                ..Options::default()
            },
            Options {
                export_batch_size: 513,
                ..Options::default()
            },
            Options {
                sentry_queue_size: 0,
                ..Options::default()
            },
        ] {
            assert!(invalid.validate().is_err());
        }
    }

    #[cfg(feature = "otlp")]
    #[test]
    fn credentials_never_travel_in_clear_text_to_a_remote_collector() {
        let with = |endpoint: &str| Options {
            otlp_endpoint: Some(String::from(endpoint)),
            otlp_authorization: Some(String::from("Bearer token")),
            ..Options::default()
        };
        assert!(
            with("http://collector.observability:4318")
                .validate()
                .is_err()
        );
        assert!(with("http://10.0.0.7:4318").validate().is_err());
        assert!(with("http://127.0.0.1:4318").validate().is_ok());
        assert!(with("http://[::1]:4318").validate().is_ok());
        assert!(with("http://localhost:4318").validate().is_ok());
        assert!(
            with("https://collector.observability:4318")
                .validate()
                .is_ok()
        );
        let plain = Options {
            otlp_endpoint: Some(String::from("http://collector.observability:4318")),
            ..Options::default()
        };
        assert!(plain.validate().is_ok());
    }
}
