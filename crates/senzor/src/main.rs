use std::path::PathBuf;

use clap::Parser;
use eframe::egui;
use senzor::icon::{APP_ID, app_icon};
use senzor::{NativeDialogs, SenzorApp};
use senzor_mcp::DEFAULT_HTTP_PORT;

#[derive(Parser)]
#[command(name = "senzor", version, about = "MQTT sensor simulator")]
struct Args {
    /// Project file to open; without it, an empty project.
    project: Option<PathBuf>,
    /// Let AI agents control this window over MCP at http://127.0.0.1:18830/mcp.
    #[arg(long)]
    mcp: bool,
    /// Serve MCP on this port instead of 18830 (implies --mcp).
    #[arg(long, value_name = "PORT")]
    mcp_port: Option<u16>,
}

fn main() -> eframe::Result {
    let args = Args::parse();
    let mcp_port = args.mcp_port.or(args.mcp.then_some(DEFAULT_HTTP_PORT));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Senzor")
            .with_app_id(APP_ID)
            .with_icon(app_icon())
            .with_inner_size([1200.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Senzor",
        options,
        Box::new(move |cc| {
            let app = SenzorApp::new(cc, args.project, Box::new(NativeDialogs));
            Ok(Box::new(match mcp_port {
                Some(port) => app.with_mcp(port),
                None => app,
            }))
        }),
    )
}
