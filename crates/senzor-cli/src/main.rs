use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use senzor_core::engine::{
    CLEAR_RETAINED_TIMEOUT, Command, Engine, EngineHandle, EngineState, InstanceStatus, Reload,
    ReloadSummary, Target, clear_retained, retained_topics,
};
use senzor_core::project::{Project, ProjectError, ProjectFile};
use senzor_mcp::{Session, serve_stdio};
use tracing_subscriber::EnvFilter;
use watch::{FileWatch, Stamp};

mod watch;

/// Exit code for an invalid project or invalid arguments.
const INVALID: u8 = 2;
/// Exit code for a `run` that ended with at least one instance Failed.
const INSTANCE_FAILED: u8 = 3;

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
        /// Apply each save of the project file: restart the devices that changed, start added
        /// ones, stop removed ones.
        #[arg(long)]
        watch: bool,
    },
    /// Delete the retained messages the project's instances leave on the broker.
    ClearRetained {
        project: PathBuf,
        /// Device IDs whose topics to clear instead of every device's.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },
    /// Print the project file's JSON Schema, for editors that check and complete TOML.
    Schema,
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
            watch,
        } => run(&project, &only, duration, watch),
        Cmd::ClearRetained { project, only } => clear(&project, &only),
        Cmd::Schema => schema(),
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
    let text = read(path)?;
    check(path, &text).map_err(|errors| {
        report(path, &errors);
        ExitCode::from(INVALID)
    })
}

fn read(path: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        eprintln!("error: cannot read {}: {e}", path.display());
        ExitCode::from(INVALID)
    })
}

/// Parses and validates `text`, the content of the project file at `path`.
fn check(path: &Path, text: &str) -> Result<Project, Vec<ProjectError>> {
    let file = ProjectFile::parse(text)?;
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let validated = file.validate(base_dir);
    if validated.errors.is_empty() {
        Ok(validated.project)
    } else {
        Err(validated.errors)
    }
}

/// Prints the schema the docs site also publishes at [`senzor_core::PROJECT_SCHEMA_URL`].
fn schema() -> ExitCode {
    let schema = senzor_core::project_schema();
    match serde_json::to_string_pretty(&schema) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
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
        let autostart = autostart_devices(project);
        if autostart.is_empty() {
            return Err("no devices to start (none has autostart = true; use --only)".into());
        }
        return Ok(autostart);
    }
    requested_devices(project, only)
}

fn autostart_devices(project: &Project) -> Vec<String> {
    project
        .devices
        .iter()
        .filter(|d| d.autostart)
        .map(|d| d.id.clone())
        .collect()
}

/// The device IDs in `--only`, trimmed, blanks left out.
fn trimmed_ids(only: &[String]) -> Vec<String> {
    only.iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect()
}

/// The devices to run after a save under `--watch`: chosen as at the start, without the errors.
/// An `--only` ID the saved project lacks starts once a later save adds the device back.
fn watched_devices(project: &Project, only: &[String]) -> Vec<String> {
    if only.is_empty() {
        autostart_devices(project)
    } else {
        trimmed_ids(only)
    }
}

/// The device IDs in a non-empty `--only`, trimmed; an error when it names none or unknown ones.
fn requested_devices(project: &Project, only: &[String]) -> Result<Vec<String>, String> {
    let requested = trimmed_ids(only);
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

fn run(path: &Path, only: &[String], duration: Option<Duration>, watch: bool) -> ExitCode {
    // Before reading, so a save while the file is read is not missed.
    let stamp = Stamp::of(path);
    let text = match read(path) {
        Ok(text) => text,
        Err(code) => return code,
    };
    let project = match check(path, &text) {
        Ok(project) => project,
        Err(errors) => {
            report(path, &errors);
            return ExitCode::from(INVALID);
        }
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
    let watch = watch.then(|| Watch {
        file: FileWatch::new(path, stamp, text),
        path: path.to_path_buf(),
        only: only.to_vec(),
        project: project.clone(),
        devices: devices.clone(),
    });
    let failed = runtime.block_on(run_engine(project, devices, duration, watch));
    if failed.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "error: {} failed: {}",
        count_noun(failed.len(), "instance"),
        failed.join(", ")
    );
    ExitCode::from(INSTANCE_FAILED)
}

/// Starts `devices`, applies the saves `watch` finds, waits for Ctrl+C, SIGTERM or `duration`,
/// then shuts the engine down. Returns the IDs of the instances that ended Failed.
async fn run_engine(
    project: Project,
    devices: Vec<String>,
    duration: Option<Duration>,
    watch: Option<Watch>,
) -> Vec<String> {
    let shutdown = shutdown_signal();
    tracing::info!(
        "connecting with MQTT {}",
        project.broker.mqtt_version.label()
    );
    let engine = Engine::spawn(project, &tokio::runtime::Handle::current(), Arc::new(|| {}));
    for id in devices {
        engine.send(Command::Start(Target::Device(id)));
    }
    let end = async {
        match duration {
            Some(duration) => {
                tokio::select! {
                    _ = shutdown => {}
                    _ = tokio::time::sleep(duration) => {}
                }
            }
            None => shutdown.await,
        }
    };
    let reloads = async {
        match watch {
            Some(watch) => watch.run(&engine).await,
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = end => {}
        _ = reloads => {}
    }
    tracing::info!("shutting down");
    let observer = engine.observer();
    engine.shutdown().await;
    observer.read(failed_instances)
}

/// `run --watch`: the project that runs and the file it came from.
struct Watch {
    file: FileWatch,
    path: PathBuf,
    only: Vec<String>,
    project: Project,
    /// The devices that run, as chosen for `project`.
    devices: Vec<String>,
}

impl Watch {
    /// Applies every save of the file to `engine`; never returns.
    async fn run(mut self, engine: &EngineHandle) {
        loop {
            let text = self.file.changed().await;
            match check(&self.path, &text) {
                Ok(project) => self.apply(project, engine),
                Err(errors) => {
                    tracing::warn!(
                        "{} has problems; the devices keep running as before",
                        self.path.display()
                    );
                    report(&self.path, &errors);
                }
            }
        }
    }

    fn apply(&mut self, project: Project, engine: &EngineHandle) {
        let devices = watched_devices(&project, &self.only);
        let reload = Reload::plan(&self.project, &self.devices, &project, &devices);
        let summary = reload.summary(&self.project, &project, &devices);
        tracing::info!(
            "reloaded {}: {}",
            self.path.display(),
            describe(&reload, &summary)
        );
        for command in reload.commands(project.clone()) {
            engine.send(command);
        }
        self.project = project;
        self.devices = devices;
    }
}

/// `broker changed; restarted a, b; started c; stopped d`, or that nothing running changed.
fn describe(reload: &Reload, summary: &ReloadSummary) -> String {
    let mut parts = Vec::new();
    if reload.broker_changed {
        parts.push("broker changed".to_string());
    }
    for (verb, ids) in [
        ("restarted", &summary.restarted),
        ("started", &summary.started),
        ("stopped", &summary.stopped),
    ] {
        if !ids.is_empty() {
            parts.push(format!("{verb} {}", ids.join(", ")));
        }
    }
    if parts.is_empty() {
        "no running device changed".into()
    } else {
        parts.join("; ")
    }
}

/// IDs of the instances whose status is Failed, in ID order. An instance that is still
/// reconnecting when the run ends stops normally and is not among them.
fn failed_instances(state: &EngineState) -> Vec<String> {
    state
        .instances
        .iter()
        .filter(|(_, instance)| matches!(instance.status, InstanceStatus::Failed { .. }))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Deletes the retained messages of the devices in `--only`, or of every device, then lists the
/// topics it cleared.
fn clear(path: &Path, only: &[String]) -> ExitCode {
    let project = match load(path) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let devices = if only.is_empty() {
        None
    } else {
        match requested_devices(&project, only) {
            Ok(devices) => Some(devices),
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::from(INVALID);
            }
        }
    };
    let topics = retained_topics(&project, devices.as_deref());
    if topics.is_empty() {
        println!("no retained topics to clear");
        return ExitCode::SUCCESS;
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let broker = &project.broker;
    match runtime.block_on(clear_retained(broker, &topics, CLEAR_RETAINED_TIMEOUT)) {
        Ok(()) => {
            for topic in &topics {
                println!("{topic}");
            }
            println!(
                "cleared {} on {}:{}",
                count_noun(topics.len(), "retained topic"),
                broker.host,
                broker.port
            );
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("error: cannot clear retained topics: {reason}");
            ExitCode::FAILURE
        }
    }
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
    use senzor_core::engine::InstanceState;

    use super::*;

    #[test]
    fn the_log_is_coloured_only_on_a_terminal_without_no_color() {
        assert!(log_colours(true, None));
        assert!(log_colours(true, Some(OsString::new())));
        assert!(!log_colours(true, Some(OsString::from("1"))));
        assert!(!log_colours(false, None));
        assert!(!log_colours(false, Some(OsString::new())));
    }

    #[test]
    fn a_reload_is_described_by_what_it_did() {
        let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let summary = ReloadSummary {
            restarted: ids(&["office", "kitchen"]),
            started: ids(&["lab"]),
            stopped: ids(&["door-1"]),
        };
        let mut reload = Reload::default();
        assert_eq!(
            describe(&reload, &summary),
            "restarted office, kitchen; started lab; stopped door-1"
        );
        reload.broker_changed = true;
        let restarted = ReloadSummary {
            restarted: ids(&["office"]),
            ..ReloadSummary::default()
        };
        assert_eq!(
            describe(&reload, &restarted),
            "broker changed; restarted office"
        );
        assert_eq!(
            describe(&Reload::default(), &ReloadSummary::default()),
            "no running device changed"
        );
    }

    #[test]
    fn failed_instances_are_those_whose_status_is_failed() {
        let mut state = EngineState::default();
        for (id, status) in [
            ("a-0", InstanceStatus::Stopped),
            (
                "a-1",
                InstanceStatus::Failed {
                    reason: "refused".into(),
                },
            ),
            ("b", InstanceStatus::Online),
            (
                "c",
                InstanceStatus::Reconnecting {
                    attempt: 3,
                    last_error: "refused".into(),
                },
            ),
            (
                "d",
                InstanceStatus::Failed {
                    reason: "panic".into(),
                },
            ),
        ] {
            let mut instance = InstanceState::new(id);
            instance.status = status;
            state.instances.insert(id.to_string(), instance);
        }
        assert_eq!(failed_instances(&state), ["a-1", "d"]);
        assert!(failed_instances(&EngineState::default()).is_empty());
    }
}
