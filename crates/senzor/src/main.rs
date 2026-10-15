use std::path::PathBuf;

use eframe::egui;
use senzor::{NativeDialogs, SenzorApp};

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Senzor")
            .with_inner_size([1200.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Senzor",
        options,
        Box::new(move |cc| Ok(Box::new(SenzorApp::new(cc, path, Box::new(NativeDialogs))))),
    )
}
