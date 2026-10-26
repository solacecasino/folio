//! The find & replace bar: hidden until Cmd+F, a slim strip above the page.

use egui::{self, Key, Margin, RichText, Ui};

use crate::doc::{Document, Pos};
use crate::theme::{Theme, color};

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub show_replace: bool,
    pub matches: Vec<(Pos, Pos)>,
    focus_query: bool,
    /// What `matches` was computed from, so it's only redone on change.
    searched: Option<(String, bool, bool, u64)>,
}

pub enum Action {
    Next,
    Previous,
    ReplaceOne,
    ReplaceAll,
    Close,
}

impl FindBar {
    /// Opens the bar (or focuses it), starting from the selected text if any.
    pub fn open(&mut self, replace: bool, selected: Option<String>) {
        self.open = true;
        self.show_replace |= replace;
        self.focus_query = true;
        if let Some(text) = selected.filter(|t| !t.is_empty() && !t.contains('\n')) {
            self.query = text;
        }
    }

    /// Recomputes matches when the query, options or document changed.
    pub fn refresh(&mut self, doc: &Document, revision: u64) {
        if !self.open {
            self.matches.clear();
            self.searched = None;
            return;
        }
        let key = (self.query.clone(), self.match_case, self.whole_word, revision);
        if self.searched.as_ref() != Some(&key) {
            self.matches = doc.find_all(&self.query, self.match_case, self.whole_word);
            self.searched = Some(key);
        }
    }

    /// The match after (or before) the caret, wrapping around.
    pub fn step(&self, from: Pos, forward: bool, selection: Option<(Pos, Pos)>) -> Option<(Pos, Pos)> {
        if forward {
            // Start after the current match so repeated Cmd+G moves on.
            let after = selection.map_or(from, |s| s.1);
            self.matches.iter().find(|m| m.0 >= after).or(self.matches.first()).copied()
        } else {
            let before = selection.map_or(from, |s| s.0);
            self.matches.iter().rev().find(|m| m.1 <= before).or(self.matches.last()).copied()
        }
    }

    /// Draws the bar. Returns the actions taken and whether one of its text
    /// fields has the keyboard (so the page shouldn't react to keys).
    pub fn show(&mut self, ui: &mut Ui, theme: &Theme, selection: Option<(Pos, Pos)>) -> (Vec<Action>, bool) {
        let mut actions = Vec::new();
        let colors = &theme.colors;
        let bg = color(colors.panel.as_deref().unwrap_or(&colors.background));
        let fg = color(colors.panel_text.as_deref().unwrap_or(&colors.status));
        let mut focused = false;

        egui::Panel::top("folio-find")
            .frame(egui::Frame::NONE.fill(bg).inner_margin(Margin::symmetric(14, 8)))
            .show(ui, |ui| {
                ui.visuals_mut().override_text_color = Some(fg);
                ui.horizontal(|ui| {
                    let query = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Find").desired_width(220.0));
                    if std::mem::take(&mut self.focus_query) {
                        query.request_focus();
                    }
                    focused |= query.has_focus();
                    let enter = query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if enter {
                        let shift = ui.input(|i| i.modifiers.shift);
                        actions.push(if shift { Action::Previous } else { Action::Next });
                        query.request_focus();
                    }

                    ui.toggle_value(&mut self.match_case, "Aa").on_hover_text("Match case");
                    ui.toggle_value(&mut self.whole_word, "W").on_hover_text("Whole words");
                    if ui.small_button("‹").on_hover_text("Previous").clicked() {
                        actions.push(Action::Previous);
                    }
                    if ui.small_button("›").on_hover_text("Next").clicked() {
                        actions.push(Action::Next);
                    }

                    let current = selection.and_then(|s| self.matches.iter().position(|m| *m == s));
                    let count = match (current, self.matches.len(), self.query.is_empty()) {
                        (_, _, true) => String::new(),
                        (_, 0, _) => "No matches".into(),
                        (Some(i), n, _) => format!("{} of {n}", i + 1),
                        (None, n, _) => format!("{n} {}", if n == 1 { "match" } else { "matches" }),
                    };
                    ui.label(RichText::new(count).small());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                            actions.push(Action::Close);
                        }
                        if !self.show_replace && ui.small_button("Replace…").clicked() {
                            self.show_replace = true;
                        }
                    });
                });

                if self.show_replace {
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut self.replacement).hint_text("Replace with").desired_width(220.0),
                        );
                        focused |= field.has_focus();
                        if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            actions.push(Action::ReplaceOne);
                            field.request_focus();
                        }
                        if ui.button("Replace").clicked() {
                            actions.push(Action::ReplaceOne);
                        }
                        if ui.button("Replace All").clicked() {
                            actions.push(Action::ReplaceAll);
                        }
                    });
                }
            });

        if focused && ui.input(|i| i.key_pressed(Key::Escape)) {
            actions.push(Action::Close);
        }
        (actions, focused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Fmt;

    #[test]
    fn steps_through_matches_and_wraps() {
        let mut doc = Document::default();
        doc.insert(Pos::default(), "a b a b a", Fmt::default(), "body");
        let mut bar = FindBar::default();
        bar.open(false, Some("a".into()));
        bar.refresh(&doc, 0);
        assert_eq!(bar.matches.len(), 3);

        let first = bar.step(Pos::default(), true, None).unwrap();
        assert_eq!(first.0, Pos::new(0, 0));
        let second = bar.step(first.1, true, Some(first)).unwrap();
        assert_eq!(second.0, Pos::new(0, 4));
        let third = bar.step(second.1, true, Some(second)).unwrap();
        let wrapped = bar.step(third.1, true, Some(third)).unwrap();
        assert_eq!(wrapped, first, "wraps to the start");
        assert_eq!(bar.step(first.0, false, Some(first)).unwrap(), third, "and backwards to the end");
    }
}
