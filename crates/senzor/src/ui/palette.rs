//! The command palette (Ctrl+K): run any action or go to any device, instance or template by
//! typing part of its name.

use eframe::egui;
use senzor_core::engine::EngineState;
use senzor_core::project::builtin::builtin_templates;

use super::Action;
use crate::document::Document;
use crate::examples::EXAMPLES;
use crate::selection::Selection;
use crate::update::Release;

/// How many matches the palette lists.
pub const SHOWN: usize = 12;

/// What choosing an entry does.
#[derive(Clone)]
pub enum Target {
    Run(fn() -> Action),
    Go(Selection),
    Example(usize),
}

/// One thing the palette can find: its name, what kind of thing it is, and what choosing it does.
#[derive(Clone)]
pub struct Entry {
    pub label: String,
    pub kind: &'static str,
    pub target: Target,
}

impl Entry {
    /// The action choosing it pushes.
    pub fn action(&self) -> Action {
        match &self.target {
            Target::Run(make) => make(),
            Target::Go(selection) => {
                Action::GoTo(crate::selection::ProblemTarget::Select(selection.clone()))
            }
            Target::Example(index) => Action::OpenExample(*index),
        }
    }
}

/// An action as the palette lists it: its name and how to make it.
type NamedAction = (&'static str, fn() -> Action);

/// The window's actions, by the name the palette shows.
const ACTIONS: &[NamedAction] = &[
    ("New project", || Action::New),
    ("Open a project…", || Action::Open),
    ("Save", || Action::Save),
    ("Save as…", || Action::SaveAs),
    ("Start all", || Action::StartAll),
    ("Stop all", || Action::StopAll),
    ("Undo", || Action::Undo),
    ("Redo", || Action::Redo),
    ("Add device", || Action::AddDevice),
    ("Add template", || Action::AddTemplate),
    ("Project settings", || Action::OpenBroker),
    ("Edit the project file as text", || {
        Action::Select(Some(crate::selection::Selection::Source))
    }),
    ("Test the broker connection", || Action::TestConnection),
    ("Clear retained messages…", || Action::AskClearRetained),
    ("Find in the message log", || Action::FindInLog),
    ("Export the message log…", || Action::ExportLog),
    ("Clear the message log", || Action::ClearLog),
    ("Settings", || Action::OpenSettings),
    ("Copy as a GitHub Actions step", || {
        Action::CopySnippet(crate::snippets::Snippet::GitHubActions)
    }),
    ("Copy as a docker-compose service", || {
        Action::CopySnippet(crate::snippets::Snippet::DockerCompose)
    }),
    ("Turn MCP on or off", || Action::ToggleMcp),
    ("What's new", || Action::ShowWhatsNew),
];

/// Everything the palette can find: the actions (with the newer release, when there is one),
/// then the project's devices, instances and templates, the built-in templates and the examples.
pub fn entries(doc: &Document, state: &EngineState, newer: Option<&Release>) -> Vec<Entry> {
    let entry = |label: &str, kind, target| Entry {
        label: label.to_string(),
        kind,
        target,
    };
    let mut entries: Vec<Entry> = ACTIONS
        .iter()
        .map(|(label, make)| entry(label, "Action", Target::Run(*make)))
        .collect();
    if let Some(release) = newer {
        let label = format!("Update: Senzor {} is out", release.version);
        entries.push(entry(
            &label,
            "Action",
            Target::Run(|| Action::OpenNewRelease),
        ));
    }
    let file = doc.file();
    for (index, device) in file.devices.iter().enumerate() {
        entries.push(entry(
            &device.id,
            "Device",
            Target::Go(Selection::Device(index)),
        ));
    }
    // The tree's instances, orphans included, in its order.
    for node in crate::tree::device_tree(file, doc.validated(), state) {
        for instance in node.instances {
            let target = Target::Go(Selection::Instance(instance.id.clone()));
            entries.push(entry(&instance.id, "Instance", target));
        }
    }
    for (index, template) in file.templates.iter().enumerate() {
        entries.push(entry(
            &template.id,
            "Template",
            Target::Go(Selection::Template(index)),
        ));
    }
    for template in builtin_templates() {
        let target = Target::Go(Selection::Builtin(template.id.clone()));
        entries.push(entry(&template.id, "Built-in template", target));
    }
    for (index, example) in EXAMPLES.iter().enumerate() {
        entries.push(entry(example.name, "Example", Target::Example(index)));
    }
    entries
}

/// The entries matching `query`, the best first, at most [`SHOWN`]: every word of the query must
/// appear in the name, in any case. Names that start with the query come first, then names with a
/// word that starts with one of the query's words.
pub fn matches<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(u8, &Entry)> = entries
        .iter()
        .filter_map(|entry| {
            let label = entry.label.to_lowercase();
            if !words.iter().all(|word| label.contains(word)) {
                return None;
            }
            let label_words: Vec<&str> = label
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            let rank = if label.starts_with(&query) {
                0
            } else if words
                .iter()
                .any(|word| label_words.iter().any(|w| w.starts_with(word)))
            {
                1
            } else {
                2
            };
            Some((rank, entry))
        })
        .collect();
    // A stable sort keeps the order of `entries` within a rank.
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().take(SHOWN).map(|(_, e)| e).collect()
}

/// What is typed into the open palette, and which match Enter runs.
#[derive(Debug, Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
}

/// What the user did in the palette this frame.
pub enum Outcome {
    Run(Box<Action>),
    Close,
}

/// Shows the open palette over the window. Up and Down move through the matches, Enter runs the
/// highlighted one, a click runs the one clicked; Escape or a click outside closes it.
pub fn show(ctx: &egui::Context, state: &mut PaletteState, all: &[Entry]) -> Option<Outcome> {
    let found = matches(all, &state.query);
    let (up, down, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
        )
    });
    if down {
        state.selected = (state.selected + 1).min(found.len().saturating_sub(1));
    }
    if up {
        state.selected = state.selected.saturating_sub(1);
    }
    let mut outcome = enter
        .then(|| {
            found
                .get(state.selected)
                .map(|e| Outcome::Run(Box::new(e.action())))
        })
        .flatten();
    let modal = egui::Modal::new(egui::Id::new("command-palette")).show(ctx, |ui| {
        ui.set_width(460.0);
        let input = ui.add(
            egui::TextEdit::singleline(&mut state.query)
                .hint_text("Type an action or the name of a device, instance or template")
                .desired_width(f32::INFINITY),
        );
        input.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Command palette")
        });
        // The box keeps the keyboard while the palette is open.
        input.request_focus();
        if input.changed() {
            state.selected = 0;
        }
        if found.is_empty() {
            ui.weak("Nothing matches.");
        }
        for (index, entry) in found.iter().enumerate() {
            let text = egui::RichText::new(&entry.label);
            let row = ui.add(
                egui::Button::selectable(index == state.selected, text)
                    .right_text(egui::RichText::new(entry.kind).weak())
                    .min_size(egui::vec2(ui.available_width(), 0.0)),
            );
            if row.clicked() {
                outcome = Some(Outcome::Run(Box::new(entry.action())));
            }
        }
    });
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Close);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use senzor_core::engine::InstanceState;
    use senzor_core::project::ProjectFile;

    use super::*;

    fn document() -> Document {
        let mut doc = Document::new_empty();
        doc.edit(|f| {
            *f = ProjectFile::parse(
                "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\ncount = 2\n\n\
                 [[templates]]\nid = \"door\"\nmessages = [{ topic = \"d\", payload = \"1\" }]\n\
                 fields.open = { generator = \"constant\", value = true }\n",
            )
            .unwrap();
        });
        doc
    }

    fn labels(found: &[&Entry]) -> Vec<String> {
        found
            .iter()
            .map(|e| format!("{}: {}", e.kind, e.label))
            .collect()
    }

    #[test]
    fn the_palette_finds_actions_and_everything_in_the_project() {
        let mut state = EngineState::default();
        state
            .instances
            .insert("old-0".into(), InstanceState::new("old"));
        let all = entries(&document(), &state, None);
        let all = labels(&all.iter().collect::<Vec<_>>());
        for wanted in [
            "Action: Stop all",
            "Device: office",
            "Instance: office-0",
            "Instance: office-1",
            "Template: door",
            "Built-in template: co2",
            "Example: Quick start",
            "Instance: old-0",
        ] {
            assert!(all.contains(&wanted.to_string()), "{wanted} in {all:?}");
        }
    }

    #[test]
    fn every_word_must_match_and_starts_rank_first() {
        let all = entries(&document(), &EngineState::default(), None);
        assert_eq!(labels(&matches(&all, "sto"))[0], "Action: Stop all");
        assert_eq!(
            labels(&matches(&all, "office 1")),
            ["Instance: office-1"],
            "both words, in any order"
        );
        assert_eq!(labels(&matches(&all, "STOP ALL")), ["Action: Stop all"]);
        assert!(matches(&all, "nothing like this").is_empty());
    }

    #[test]
    fn a_newer_release_can_be_found_as_an_update() {
        let release = Release {
            version: "9.9.9".into(),
            url: "https://github.com/kankaj/Senzor/releases/tag/v9.9.9".into(),
        };
        let all = entries(&document(), &EngineState::default(), Some(&release));
        let found = matches(&all, "update");
        assert_eq!(labels(&found), ["Action: Update: Senzor 9.9.9 is out"]);
        assert!(matches!(found[0].action(), Action::OpenNewRelease));
        let without = entries(&document(), &EngineState::default(), None);
        assert!(matches(&without, "update").is_empty());
    }

    #[test]
    fn an_empty_query_lists_the_actions_first_and_no_more_than_shown() {
        let all = entries(&document(), &EngineState::default(), None);
        let found = matches(&all, "");
        assert_eq!(found.len(), SHOWN);
        assert_eq!(found[0].kind, "Action");
    }
}
