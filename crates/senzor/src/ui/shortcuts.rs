use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};

use super::Action;

pub const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
pub const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
pub const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
pub const START_ALL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
pub const STOP_ALL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F5);
pub const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);
pub const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
pub const REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
/// Redo as most Windows and Linux programs know it.
pub const REDO_Y: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);

/// Each shortcut with the action it stands for, the most specific first: egui ignores an extra
/// Shift when it matches, so Ctrl+S would also take Ctrl+Shift+S.
const ALL: [(KeyboardShortcut, fn() -> Action); 10] = [
    (REDO, || Action::Redo),
    (UNDO, || Action::Undo),
    (REDO_Y, || Action::Redo),
    (SAVE_AS, || Action::SaveAs),
    (SAVE, || Action::Save),
    (NEW, || Action::New),
    (OPEN, || Action::Open),
    (STOP_ALL, || Action::StopAll),
    (START_ALL, || Action::StartAll),
    (SETTINGS, || Action::OpenSettings),
];

/// The actions for the shortcuts pressed this frame, which it takes from egui's input. While a
/// text field has the keyboard, undo and redo stay with the field: it takes back what was typed
/// into it, and so the project's edits from it.
pub fn pressed(ctx: &egui::Context) -> Vec<Action> {
    let typing = ctx.egui_wants_keyboard_input();
    ctx.input_mut(|input| {
        ALL.iter()
            .filter(|(shortcut, _)| !(typing && is_undo_or_redo(shortcut)))
            .filter(|(shortcut, _)| input.consume_shortcut(shortcut))
            .map(|(_, action)| action())
            .collect()
    })
}

fn is_undo_or_redo(shortcut: &KeyboardShortcut) -> bool {
    [UNDO, REDO, REDO_Y].contains(shortcut)
}

/// `text` followed by `shortcut` in parentheses, as the tooltips show it.
pub fn tip(ctx: &egui::Context, text: &str, shortcut: &KeyboardShortcut) -> String {
    format!("{text} ({})", ctx.format_shortcut(shortcut))
}
