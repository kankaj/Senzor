//! egui drawing. Panels read the document and engine state and push [`Action`]s; the app applies them.

pub mod bottom;
pub mod broker;
pub mod device;
pub mod instance;
pub mod left_panel;
pub mod modals;
pub mod settings;
pub mod shortcuts;
pub mod switch;
pub mod template;
pub mod top_bar;
pub mod welcome;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Color32, TextEdit};
use senzor_core::engine::{Command, InstanceStatus, LogEntry};
use senzor_core::project::{Location, ProjectFile};
use senzor_core::value::Value;

use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{Removal, Rename, ValueKind, parse_value};
use crate::selection::{ProblemTarget, Selection};
use crate::settings::Theme;

pub const ERROR: Color32 = Color32::from_rgb(220, 70, 70);
pub const WARNING: Color32 = Color32::from_rgb(220, 160, 40);
const ONLINE: Color32 = Color32::from_rgb(60, 180, 90);
const STOPPED: Color32 = Color32::from_gray(140);

/// Hover hint of a disabled Start: the Problems tab lists what blocks the device.
pub const FIX_PROBLEMS_FIRST: &str = "Fix the problems in the Problems tab first";

/// Something the user asked for this frame. The app applies a frame's actions in [`phase`]
/// order, so a pending edit or rename always lands before a delete, New, Open or Save.
pub enum Action {
    New,
    Open,
    Save,
    SaveAs,
    StartAll,
    StopAll,
    Engine(Command),
    Select(Option<Selection>),
    /// A form's edit, built by [`replace_if_unchanged`].
    Edit(Box<dyn FnOnce(&mut ProjectFile)>),
    Rename(Rename),
    Remove(Removal),
    AddDevice,
    AddTemplate,
    AskCopyBuiltin(String),
    OpenBroker,
    OpenSettings,
    Undo,
    Redo,
    /// Open a project from the Recent menu.
    OpenRecent(PathBuf),
    ClearRecent,
    SetTheme(Theme),
    GoTo(ProblemTarget),
    /// Turn the MCP endpoint on or off.
    ToggleMcp,
}

/// Order in which a frame's actions are applied: edits, then renames, then removals, then the rest.
pub fn phase(action: &Action) -> u8 {
    match action {
        Action::Edit(_) => 0,
        Action::Rename(_) => 1,
        Action::Remove(_) => 2,
        _ => 3,
    }
}

/// Applies an edit, rename or removal to the document and reports whether it changed;
/// hands every other action back.
/// `typing`: the text field being typed into, whose edits make one undo step.
pub fn apply_to_document(
    doc: &mut Document,
    action: Action,
    typing: Option<u64>,
) -> Result<bool, Box<Action>> {
    match action {
        Action::Edit(f) => Ok(doc.edit_grouped(typing, f)),
        Action::Rename(rename) => Ok(doc.edit(|f| {
            // Dropping a refused rename is intended: its name box checked it, so `apply` only
            // refuses it when an action applied earlier removed, moved or renamed what it names.
            let _ = rename.apply(f);
        })),
        Action::Remove(removal) => Ok(doc.edit(|f| removal.apply(f))),
        other => Err(Box::new(other)),
    }
}

/// The [`Action::Edit`] that stores `edited` in the item `slot` finds, or `None` when the form
/// left the item as `original`. Forms edit a copy of what they show, so the edit is a
/// compare-and-swap: it only applies while the slot still holds `original`, and an edit that
/// arrives after the item changed or moved is dropped instead of overwriting another item.
pub fn replace_if_unchanged<T: Clone + PartialEq + 'static>(
    original: &T,
    edited: T,
    slot: impl for<'f> FnOnce(&'f mut ProjectFile) -> Option<&'f mut T> + 'static,
) -> Option<Action> {
    if edited == *original {
        return None;
    }
    let original = original.clone();
    Some(Action::Edit(Box::new(move |f| {
        if let Some(current) = slot(f)
            && *current == original
        {
            *current = edited;
        }
    })))
}

/// What to do once unsaved changes are saved or discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    New,
    /// Ask which file to open.
    Open,
    /// Open this file, chosen from the recent projects.
    OpenPath(PathBuf),
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    Unsaved(Pending),
    OpenFailed {
        path: String,
        message: String,
    },
    CopyBuiltin {
        builtin: String,
        new_id: String,
        error: Option<String>,
    },
    /// Saving would replace `path`, a file the save dialog did not ask about (it got `.toml`
    /// added to the name the user typed). `then` continues once the project is saved.
    ConfirmReplace {
        path: PathBuf,
        then: Option<Pending>,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    Devices,
    Templates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomTab {
    #[default]
    Log,
    Problems,
}

/// GUI-only state that is not part of the project.
#[derive(Default)]
pub struct UiState {
    pub selection: Option<Selection>,
    pub left_tab: LeftTab,
    pub bottom_tab: BottomTab,
    pub buffers: Buffers,
    /// A valid rename typed into a name box but not committed yet (set again every frame).
    pub pending_rename: Option<Rename>,
    /// Pin box text per (instance ID, field name).
    pub pin_inputs: HashMap<(String, String), String>,
    pub log_filter: String,
    /// A frozen copy of the log while it is paused.
    pub log_frozen: Option<VecDeque<LogEntry>>,
    pub broker_open: bool,
    pub settings_open: bool,
    pub modal: Option<Modal>,
}

impl UiState {
    /// Forgets everything tied to the previous document.
    pub fn reset(&mut self) {
        let left_tab = self.left_tab;
        *self = UiState::default();
        self.left_tab = left_tab;
    }
}

fn status_color(status: &InstanceStatus) -> Color32 {
    match status {
        InstanceStatus::Online => ONLINE,
        InstanceStatus::Connecting | InstanceStatus::Reconnecting { .. } => WARNING,
        InstanceStatus::Stopped => STOPPED,
        InstanceStatus::Failed { .. } => ERROR,
    }
}

/// A small filled circle in the status colour.
pub fn status_dot(ui: &mut egui::Ui, status: &InstanceStatus) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 4.0, status_color(status));
}

/// A single-line box over a typed value, whose text is kept in [`Buffers`] under `key` (also
/// the box's ID salt). Changed text that parses is written to the value and stays as typed;
/// text that does not parse stays in the box with the parse error after it and leaves the
/// value unchanged.
pub struct ParsedEdit<'a> {
    key: &'a str,
    width: f32,
    hint: Option<&'static str>,
}

impl<'a> ParsedEdit<'a> {
    pub fn new(key: &'a str, width: f32) -> Self {
        Self {
            key,
            width,
            hint: None,
        }
    }

    /// Text shown while the box is empty.
    pub fn hint(self, hint: &'static str) -> Self {
        Self {
            hint: Some(hint),
            ..self
        }
    }

    /// Draws the box for `value`, written as text by `to_text` and read back by `parse`.
    pub fn show<T>(
        self,
        ui: &mut egui::Ui,
        buffers: &mut Buffers,
        value: &mut T,
        to_text: impl Fn(&T) -> String,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> egui::Response {
        let buffer = buffers.get(self.key, &to_text(value));
        let mut edit = TextEdit::singleline(&mut buffer.text)
            .id_salt(self.key)
            .desired_width(self.width);
        if let Some(hint) = self.hint {
            edit = edit.hint_text(hint);
        }
        let response = ui.add(edit);
        match parse(&buffer.text) {
            Ok(parsed) => {
                if response.changed() {
                    buffer.committed(&to_text(&parsed));
                    *value = parsed;
                }
            }
            Err(e) => {
                ui.colored_label(ERROR, e);
            }
        }
        response
    }
}

/// A duration typed like `10s`, `500ms`, `1h`. Invalid text stays in the box with a hint
/// and leaves `value` unchanged.
pub fn duration_edit(
    ui: &mut egui::Ui,
    buffers: &mut Buffers,
    key: &str,
    value: &mut Duration,
) -> egui::Response {
    ParsedEdit::new(key, 90.0).show(
        ui,
        buffers,
        value,
        |value| humantime::format_duration(*value).to_string(),
        |text| humantime::parse_duration(text).map_err(|_| "use e.g. 10s, 500ms, 1h".to_string()),
    )
}

/// An optional duration: empty means none, otherwise typed like `10s`, `500ms`, `1h`. Invalid
/// text stays in the box with a hint and leaves `value` unchanged.
pub fn optional_duration_edit(
    ui: &mut egui::Ui,
    buffers: &mut Buffers,
    key: &str,
    value: &mut Option<Duration>,
) -> egui::Response {
    ParsedEdit::new(key, 90.0).hint("none").show(
        ui,
        buffers,
        value,
        |value| value.map_or_else(String::new, |d| humantime::format_duration(d).to_string()),
        |text| {
            if text.trim().is_empty() {
                return Ok(None);
            }
            humantime::parse_duration(text)
                .map(Some)
                .map_err(|_| "use e.g. 10s, 1m, 1h".to_string())
        },
    )
}

/// A param or `constant` value: a type selector and a box read as that type, so typing never
/// changes the type. Invalid text stays in the box with a hint and leaves `value` unchanged.
/// Another type can only be chosen when the box's text reads as it; choosing it stores that
/// value and keeps the text. Returns the box.
pub fn value_edit(
    ui: &mut egui::Ui,
    buffers: &mut Buffers,
    key: &str,
    value: &mut Value,
) -> egui::Response {
    let current = ValueKind::of(value);
    let buffer = buffers.get(key, &value.to_input());
    let mut chosen = current;
    egui::ComboBox::from_id_salt(format!("{key}/kind"))
        .selected_text(current.label())
        .width(70.0)
        .show_ui(ui, |ui| {
            for option in ValueKind::ALL {
                // The current type stays selectable; why the text does not fit it is shown
                // under the box.
                let fits = if option == current {
                    Ok(())
                } else {
                    parse_value(option, &buffer.text).map(drop)
                };
                let mut item = ui.add_enabled(
                    fits.is_ok(),
                    egui::Button::selectable(option == current, option.label()),
                );
                if let Err(why) = fits {
                    item = item.on_disabled_hover_text(why);
                }
                if item.clicked() {
                    chosen = option;
                }
            }
        });
    if chosen != current
        && let Ok(parsed) = parse_value(chosen, &buffer.text)
    {
        *value = parsed;
        // Keep the text as typed, e.g. `42x` when a number box switches to text.
        buffer.committed(&value.to_input());
    }
    let kind = ValueKind::of(value);
    ParsedEdit::new(key, 140.0).show(ui, buffers, value, Value::to_input, |text| {
        parse_value(kind, text)
    })
}

/// A name box that commits on Enter or when it loses focus, as the [`Rename`] that `rename`
/// builds from the trimmed text. Shows why a name cannot be used, and records a valid but
/// uncommitted rename in `ui_state.pending_rename`. Returns the box and the rename to commit.
pub fn name_edit(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    key: &str,
    current: &str,
    file: &ProjectFile,
    rename: impl Fn(String) -> Rename,
) -> (egui::Response, Option<Rename>) {
    let buffer = ui_state.buffers.get(key, current);
    let response = ui.add(
        TextEdit::singleline(&mut buffer.text)
            .id_salt(key)
            .desired_width(160.0),
    );
    let to = buffer.text.trim().to_string();
    if to == current {
        return (response, None);
    }
    let candidate = rename(to);
    match candidate.check(file) {
        Ok(()) if response.lost_focus() => (response, Some(candidate)),
        Ok(()) => {
            ui_state.pending_rename = Some(candidate);
            (response, None)
        }
        Err(e) => {
            ui.colored_label(ERROR, e);
            (response, None)
        }
    }
}

pub use senzor_core::project::NEEDS_MQTT5;

/// Shows an MQTT 5-only control. On MQTT 3.1.1 it stays usable only while it holds a value, so the
/// user can clear what validation reports, and the reason shows next to it.
pub fn mqtt5_control<R>(
    ui: &mut egui::Ui,
    mqtt5: bool,
    has_value: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let inner = ui.add_enabled_ui(mqtt5 || has_value, add);
        if !mqtt5 {
            inner.response.on_disabled_hover_text(NEEDS_MQTT5);
            ui.weak(NEEDS_MQTT5);
        }
        inner.inner
    })
    .inner
}

/// An optional text value: an empty box means `None`.
pub fn optional_text(ui: &mut egui::Ui, value: &mut Option<String>, hint: &str) -> egui::Response {
    let mut text = value.clone().unwrap_or_default();
    let response = ui.add(TextEdit::singleline(&mut text).hint_text(hint.to_string()));
    if response.changed() {
        *value = (!text.is_empty()).then_some(text);
    }
    response
}

/// Validation errors located at `location`, in red; returns whether there were any. Drawn in
/// their own scope so a changing number of errors does not shift the IDs (and focus) of the
/// widgets below.
pub fn errors_at(ui: &mut egui::Ui, doc: &Document, location: &Location) -> bool {
    ui.push_id("errors", |ui| {
        let mut any = false;
        for error in doc
            .validated()
            .errors
            .iter()
            .filter(|e| &e.location == location)
        {
            ui.colored_label(ERROR, &error.message);
            any = true;
        }
        any
    })
    .inner
}

/// Where the documentation lives.
pub const DOCS_URL: &str = "https://senzor.kankaj.cz/docs";
/// Where the source code lives.
pub const SOURCE_URL: &str = "https://github.com/kankaj/Senzor";

/// A recent project: its file name and, smaller, its folder. Screen readers and tests know it
/// by its full path.
pub fn recent_project_button(ui: &mut egui::Ui, path: &Path) -> egui::Response {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let response = ui.button((
        egui::RichText::new(name),
        egui::RichText::new(folder).weak().small(),
    ));
    let full = path.display().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &full));
    response
}

/// Sets `ctx`'s fonts: egui's text fonts, and Phosphor as the only icon font, a fallback after the
/// text font, so the constants in [`egui_phosphor::regular`] can go into any label or button.
pub fn install_icon_font(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
}

fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for emoji in ["NotoEmoji-Regular", "emoji-icon-font"] {
        fonts.font_data.remove(emoji);
        for names in fonts.families.values_mut() {
            names.retain(|name| name != emoji);
        }
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts
}

#[cfg(test)]
mod font_tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Phosphor is the only icon font: egui's emoji fonts are left out, so every icon has the
    /// same style.
    #[test]
    fn the_fonts_are_the_text_fonts_and_phosphor() {
        let fonts = fonts();
        let loaded: BTreeSet<&str> = fonts.font_data.keys().map(String::as_str).collect();
        assert_eq!(loaded, BTreeSet::from(["Hack", "Ubuntu-Light", "phosphor"]));
        for (family, names) in &fonts.families {
            for name in names {
                assert!(fonts.font_data.contains_key(name), "{family:?} uses {name}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use eframe::egui;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use senzor_core::engine::{InstanceState, InstanceStatus};
    use senzor_core::project::ProjectFile;

    use super::{Action, UiState, apply_to_document, phase};
    use crate::document::Document;

    /// A document and the GUI state one panel needs, for driving that panel with egui_kittest.
    pub struct Fixture {
        pub doc: Document,
        pub ui: UiState,
        pub actions: Vec<Action>,
        /// Actions other than edits, renames and removals, in the order the panel pushed them.
        pub other: Vec<Action>,
    }

    impl Fixture {
        pub fn new(src: &str) -> Self {
            let mut doc = Document::new_empty();
            doc.edit(|f| *f = ProjectFile::parse(src).unwrap());
            Self {
                doc,
                ui: UiState::default(),
                actions: Vec::new(),
                other: Vec::new(),
            }
        }

        /// A harness whose every frame draws with `draw` and then applies the frame's edits;
        /// the first frames have run.
        pub fn harness(
            self,
            mut draw: impl FnMut(&mut egui::Ui, &mut Fixture) + 'static,
        ) -> Harness<'static, Fixture> {
            let mut harness = Harness::new_ui_state(
                move |ui, fx: &mut Fixture| {
                    draw(ui, fx);
                    fx.apply_edits();
                },
                self,
            );
            harness.run();
            harness
        }

        /// Applies the frame's edits, renames and removals in phase order, as the app does.
        pub fn apply_edits(&mut self) {
            let mut actions: Vec<Action> = self.actions.drain(..).collect();
            actions.sort_by_key(phase);
            for action in actions {
                if let Err(other) = apply_to_document(&mut self.doc, action, None) {
                    self.other.push(*other);
                }
            }
        }
    }

    /// Focuses the widget labelled `label`, types `text` into it and runs the frames that follow.
    pub fn type_into_label(h: &mut Harness<'static, Fixture>, label: &str, text: &str) {
        let node = h.get_by_label(label);
        node.focus();
        node.type_text(text);
        h.run();
    }

    /// Like [`type_into_label`], for the first widget whose value is `value`.
    pub fn type_into_value(h: &mut Harness<'static, Fixture>, value: &str, text: &str) {
        let node = h.get_all_by_value(value).next().unwrap();
        node.focus();
        node.type_text(text);
        h.run();
    }

    /// Types each of `texts` into the focused widget in a frame of its own, as a user typing one
    /// character at a time does.
    pub fn type_per_frame(h: &mut Harness<'static, Fixture>, texts: &[&str]) {
        for text in texts {
            h.event(egui::Event::Text(text.to_string()));
            h.run();
        }
    }

    /// Checks that the widget labelled `item` is disabled, shows `hint` on hover and pushes no
    /// engine command when clicked.
    pub fn assert_disabled_with_hint(h: &mut Harness<'static, Fixture>, item: &str, hint: &str) {
        let node = h.get_by_label(item);
        assert!(node.accesskit_node().is_disabled(), "`{item}` is disabled");
        node.hover();
        h.run();
        assert!(h.query_by_label(hint).is_some(), "hint `{hint}` shown");

        h.get_by_label(item).click();
        h.run();
        let fx = h.state();
        assert!(
            !fx.actions
                .iter()
                .chain(&fx.other)
                .any(|a| matches!(a, Action::Engine(_)))
        );
    }

    /// An engine instance of `device` in `status`.
    pub fn instance(device: &str, status: InstanceStatus) -> InstanceState {
        let mut instance = InstanceState::new(device);
        instance.status = status;
        instance
    }
}
