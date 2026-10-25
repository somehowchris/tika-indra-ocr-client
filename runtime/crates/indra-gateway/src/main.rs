use clap::{CommandFactory, Parser};
use indra_gateway::{
    commands,
    configuration::{self, Cli, Command, Settings},
    error::{self, Error},
    service,
};
use std::time::Duration;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> miette::Result<()> {
    run().map_err(miette::Report::new)
}

fn run() -> Result<(), Error> {
    configuration::debug_dotenv()?;
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "indra-gateway",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
        Some(Command::Openapi) => {
            serde_json::to_writer_pretty(std::io::stdout().lock(), &service::api_document())
                .map_err(|_| Error::Configuration("cannot serialize OpenAPI"))?;
            return Ok(());
        }
        _ => {}
    }
    let settings = Settings::load(cli.config, cli.overrides)?;
    match cli.command {
        Some(Command::Probe { kind }) => commands::probe(settings.gateway.listen, kind),
        Some(Command::Drain) => commands::drain(settings.gateway.listen),
        Some(Command::CheckConfig) => {
            println!(
                "Gateway configuration valid: active={}, connections={}, request_seconds={}",
                settings.gateway.max_active,
                settings.gateway.max_connections,
                settings.gateway.request_timeout_seconds
            );
            Ok(())
        }
        None | Some(Command::Serve) => {
            let telemetry = indra_telemetry::initialize(&settings.telemetry)?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|source| error::io("creating Tokio runtime", source))?;
            let served = runtime.block_on(service::serve(settings));
            runtime.shutdown_timeout(Duration::from_secs(1));
            let exported = telemetry.shutdown();
            if served.is_err() && exported.is_err() {
                eprintln!("Gateway telemetry flush also exceeded its budget");
            }
            served?;
            exported?;
            Ok(())
        }
        Some(Command::Completions { .. } | Command::Openapi) => {
            unreachable!("handled before config")
        }
    }
}
