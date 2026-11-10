//! The Senzor GUI: an egui editor and runner for Senzor project files.

mod app;
mod buffers;
mod dialogs;
mod edits;
mod examples;
mod format;
pub mod i18n;
pub mod icon;
mod log_view;
pub mod notify;
mod preview;
mod rate;
mod selection;
pub mod settings;
mod snippets;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod tree;
mod ui;
pub mod update;
mod whats_new;

pub use app::{McpStatus, SenzorApp};
pub use dialogs::{FileDialogs, NativeDialogs};
pub use notify::{DesktopNotifier, Notifier};
/// The project being edited; it lives in `senzor-core` so every editor shares it.
pub use senzor_core::project::document;
