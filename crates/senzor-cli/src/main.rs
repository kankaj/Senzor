use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use senzor_core::engine::{Command, Engine, Target};
use senzor_core::project::{Project, ProjectError, ProjectFile};
use senzor_mcp::{Session, serve_stdio};
use tracing_subscriber::EnvFilter;

/// Exit code for an invalid project or invalid arguments.
const INVALID: u8 = 2;

#[derive(Parser)]
#[command(name = "senzor-cli", version, about = "Headless MQTT sensor simulator")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
    /// Log every publish (debug level). RUST_LOG overrides.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
}

#[derive(Subcommand)]
enum Cmd {
    /// Load and validate a project file, print every problem.
    Validate { project: PathBuf },
    /// Run a project until Ctrl+C or --duration elapses.
    Run {
        project: PathBuf,
        /// Device IDs to start instead of those with `autostart = true`.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Stop after this long, e.g. `30s` or `10m`.
        #[arg(long, value_parser = humantime::parse_duration)]
        duration: Option<Duration>,
    },
    /// Serve the Model Context Protocol on stdin/stdout, so an AI agent can control Senzor.
    Mcp {
        /// Project file to open; without it, an empty project.
        project: Option<PathBuf>,
        /// Do not start the devices with `autostart = true`.
        #[arg(long)]
        no_autostart: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    match cli.command {
        Cmd::Validate { project } => validate(&project),
        Cmd::Run {
            project,
            only,
            duration,
        } => run(&project, &only, duration),
        Cmd::Mcp {
            project,
            no_autostart,
        } => mcp(project.as_deref(), !no_autostart),
    }
}

fn init_logging(verbose: u8) {
    let level = if verbose == 0 { "info" } else { "debug" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("warn,senzor_core={level},senzor_cli={level}")));
    let colours = log_colours(
        std::io::stderr().is_terminal(),
        std::env::var_os("NO_COLOR"),
    );
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(colours)
        .init();
}

/// Colours only for a terminal, and not when `NO_COLOR` is set to a non-empty value
/// (<https://no-color.org>), so CI logs and `docker logs` get plain text.
fn log_colours(stderr_is_terminal: bool, no_color: Option<OsString>) -> bool {
    stderr_is_terminal && no_color.is_none_or(|value| value.is_empty())
}

fn report(path: &Path, errors: &[ProjectError]) {
    for error in errors {
        eprintln!("{}: {error}", path.display());
    }
}

/// Reads, parses and validates; prints problems and returns the exit code on failure.
fn load(path: &Path) -> Result<Project, ExitCode> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: cannot read {}: {e}", path.display());
        ExitCode::from(INVALID)
    })?;
    let file = ProjectFile::parse(&text).map_err(|errors| {
        report(path, &errors);
        ExitCode::from(INVALID)
    })?;
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let validated = file.validate(base_dir);
    if !validated.errors.is_empty() {
        report(path, &validated.errors);
        return Err(ExitCode::from(INVALID));
    }
    Ok(validated.project)
}

fn validate(path: &Path) -> ExitCode {
    match load(path) {
        Ok(project) => {
            println!(
                "{}: OK ({}, {}, MQTT {})",
                path.display(),
                count_noun(project.devices.len(), "device"),
                count_noun(project.instances().count(), "instance"),
                project.broker.mqtt_version.label()
            );
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// `1 device`, `2 devices`: the noun takes an `s` unless the count is exactly 1.
fn count_noun(count: usize, noun: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{plural}")
}

/// Device IDs to start: `--only` (trimmed) or every device with `autostart = true`; never empty.
fn select_devices(project: &Project, only: &[String]) -> Result<Vec<String>, String> {
    if only.is_empty() {
        let autostart: Vec<String> = project
            .devices
            .iter()
            .filter(|d| d.autostart)
            .map(|d| d.id.clone())
            .collect();
        if autostart.is_empty() {
            return Err("no devices to start (none has autostart = true; use --only)".into());
        }
        return Ok(autostart);
    }
    let requested: Vec<String> = only
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if requested.is_empty() {
        return Err("--only names no devices".into());
    }
    let unknown: Vec<&str> = requested
        .iter()
        .filter(|id| project.device(id).is_none())
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        Ok(requested)
    } else {
        Err(format!("unknown device id(s): {}", unknown.join(", ")))
    }
}

fn run(path: &Path, only: &[String], duration: Option<Duration>) -> ExitCode {
    let project = match load(path) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let devices = match select_devices(&project, only) {
        Ok(devices) => devices,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(INVALID);
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run_engine(project, devices, duration));
    ExitCode::SUCCESS
}

/// Starts `devices`, waits for Ctrl+C, SIGTERM or `duration`, then shuts the engine down.
async fn run_engine(project: Project, devices: Vec<String>, duration: Option<Duration>) {
    let shutdown = shutdown_signal();
    tracing::info!(
        "connecting with MQTT {}",
        project.broker.mqtt_version.label()
    );
    let engine = Engine::spawn(project, &tokio::runtime::Handle::current(), Arc::new(|| {}));
    for id in devices {
        engine.send(Command::Start(Target::Device(id)));
    }
    match duration {
        Some(duration) => {
            tokio::select! {
                _ = shutdown => {}
                _ = tokio::time::sleep(duration) => {}
            }
        }
        None => shutdown.await,
    }
    tracing::info!("shutting down");
    engine.shutdown().await;
}

/// Serves MCP until the client disconnects, Ctrl+C or SIGTERM. stdout carries only the protocol.
fn mcp(path: Option<&Path>, autostart: bool) -> ExitCode {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async {
        let shutdown = shutdown_signal();
        let session = Session::new(&tokio::runtime::Handle::current(), path, autostart);
        match (session.open_error(), path) {
            (Some(error), _) => eprintln!("warning: {error}; starting with an empty project"),
            (None, Some(path)) => report(path, &session.document().validated().errors),
            (None, None) => {}
        }
        let session = serve_stdio(session, shutdown).await;
        if session.document().is_dirty() {
            eprintln!("warning: the project has unsaved changes; they are discarded");
        }
        tracing::info!("shutting down");
        session.shutdown().await;
    });
    ExitCode::SUCCESS
}

/// Completes on Ctrl+C or, on Unix, SIGTERM (what `docker stop` and systemd send).
/// The SIGTERM handler is registered when this is called, so call it before starting devices.
fn shutdown_signal() -> impl Future<Output = ()> {
    #[cfg(unix)]
    let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
    async move {
        #[cfg(unix)]
        if let Ok(mut terminate) = terminate {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_coloured_only_on_a_terminal_without_no_color() {
        assert!(log_colours(true, None));
        assert!(log_colours(true, Some(OsString::new())));
        assert!(!log_colours(true, Some(OsString::from("1"))));
        assert!(!log_colours(false, None));
        assert!(!log_colours(false, Some(OsString::new())));
    }
}
