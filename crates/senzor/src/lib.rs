//! The Senzor GUI: an egui editor and runner for Senzor project files.

mod app;
mod buffers;
mod dialogs;
mod edits;
mod format;
pub mod icon;
mod preview;
mod selection;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod tree;
mod ui;

pub use app::{McpStatus, SenzorApp};
pub use dialogs::{FileDialogs, NativeDialogs};
/// The project being edited; it lives in `senzor-core` so every editor shares it.
pub use senzor_core::project::document;
