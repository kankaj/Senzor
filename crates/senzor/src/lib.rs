//! The Senzor GUI: an egui editor and runner for Senzor project files.

mod app;
mod buffers;
mod dialogs;
pub mod document;
mod edits;
mod format;
mod preview;
mod selection;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod tree;
mod ui;

pub use app::SenzorApp;
pub use dialogs::{FileDialogs, NativeDialogs};
