use clap::{CommandFactory, Parser};
use indra_server::{
    commands,
    configuration::{self, Cli, Command, ModelCommand, Settings},
    error::StartupError,
    service,
};
use std::time::Duration;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> miette::Result<()> {
    run().map_err(miette::Report::new)
}

fn models(
    settings: &Settings,
    action: ModelCommand,
) -> Result<indra_models::CacheReport, StartupError> {
    let (name, result) = match action {
        ModelCommand::Fetch => (
            "fetch",
            indra_models::fetch_with(
                &settings.models.directory,
                settings.engine.math,
                &settings.models.fetch_options(),
            ),
        ),
        ModelCommand::Verify => (
            "verify",
            indra_models::verify(&settings.models.directory, settings.engine.math),
        ),
    };
    match result {
        Ok(report) => {
            tracing::info!(
                event = "models_verified",
                action = name,
                model_revision = indra_models::MODEL_REVISION,
                math = settings.engine.math,
                files = report.files,
                bytes = report.bytes,
                downloaded = report.downloaded,
                reused = report.reused,
                "Model cache verified"
            );
            Ok(report)
        }
        Err(error) => {
            let class = error.class();
            indra_telemetry::report_failure(&indra_telemetry::FailureContext {
                class,
                reason: None,
                phase: indra_telemetry::FailurePhase::Startup,
                request_id: None,
                worker: None,
                elapsed: Duration::ZERO,
            });
            Err(StartupError::Models(error))
        }
    }
}

fn run() -> Result<(), StartupError> {
    configuration::debug_dotenv()?;
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "indra-server",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
        Some(Command::Openapi) => {
            serde_json::to_writer_pretty(std::io::stdout().lock(), &service::api_document())
                .map_err(|_| StartupError::OpenApi)?;
            return Ok(());
        }
        _ => {}
    }
    let settings = Settings::load(cli.config, cli.overrides)?;
    match cli.command {
        Some(Command::CheckConfig) => println!(
            "Configuration valid: workers={}, threads={}, cv_threads={}, batch_size={}, math={}",
            settings.server.workers,
            settings.engine.threads,
            settings.engine.engine_config().cv_threads,
            settings.engine.batch_size,
            settings.engine.math
        ),
        Some(Command::Probe { kind }) => commands::probe(settings.server.listen, kind)?,
        Some(Command::Drain) => commands::drain(settings.server.listen)?,
        Some(Command::Models { action }) => {
            // The init container and verify probes run this path; their
            // failures must reach the same JSON stream and Sentry as serving.
            let telemetry = indra_telemetry::initialize(&settings.telemetry)?;
            let result = models(&settings, action);
            let exported = telemetry.shutdown();
            let report = result?;
            exported?;
            println!(
                "{{\"revision\":\"{}\",\"files\":{},\"bytes\":{},\"reused\":{},\"downloaded\":{}}}",
                indra_models::MODEL_REVISION,
                report.files,
                report.bytes,
                report.reused,
                report.downloaded
            );
        }
        None | Some(Command::Serve) => {
            // dotenv/config and blocking exporter setup precede the runtime.
            let telemetry = indra_telemetry::initialize(&settings.telemetry)?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let result = runtime.block_on(service::serve(settings));
            runtime.shutdown_timeout(Duration::from_secs(1));
            let exported = telemetry.shutdown();
            if result.is_err() && exported.is_err() {
                eprintln!("Telemetry flush also exceeded its shutdown budget");
            }
            result?;
            exported?;
        }
        Some(Command::Completions { .. } | Command::Openapi) => {
            unreachable!("handled before configuration loading")
        }
    }
    Ok(())
}
