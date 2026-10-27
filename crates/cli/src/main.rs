//! `specsentry serve` runs the web UI + API; `specsentry check` gates CI.

mod check;
mod healthcheck;
mod serve;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "specsentry",
    version,
    about = "Catch breaking OpenAPI contract changes before release."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the web UI and JSON API (configured through environment variables).
    Serve(serve::ServeArgs),
    /// Compare two OpenAPI JSON documents and fail on breaking changes.
    Check(check::CheckArgs),
    /// Probe a running server's health endpoint (for container HEALTHCHECK).
    Healthcheck(healthcheck::HealthcheckArgs),
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check(args) => check::run(&args),
        Command::Healthcheck(args) => healthcheck::run(&args),
        Command::Serve(args) => match serve::run(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                // Logging may not be initialised yet (e.g. invalid config).
                eprintln!("error: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}
