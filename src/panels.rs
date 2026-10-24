//! Panels around the page: an outline, text panels written in the formatting
//! language, and panels filled by components. They're placed by
//! `[[panels]]` entries in settings.toml; moving or resizing one here writes
//! the change back there.

use eframe::egui::{self, Margin, RichText, Ui};

use crate::components::Host;
use crate::doc::Document;
use crate::format::Format;
use crate::keys::Command;
use crate::settings::{PanelKind, PanelSpec, Side};
use crate::theme::{Theme, color};

pub enum Action {
    Goto(usize),
    RestoreVersion(std::path::PathBuf),
    Run(Command),
    Move(usize, Side),
    Close(usize),
    Resize(usize, f32),
}

pub struct Context<'a> {
    pub doc: &'a Document,
    pub theme: &'a Theme,
    pub cursor_para: usize,
    /// Parsed `lines` for each panel, by panel index.
    pub text_lines: &'a [Vec<Format>],
    pub fields: &'a dyn Fn(&str) -> Option<String>,
    pub host: &'a Host,
    /// This document's saved versions, newest first.
    pub versions: &'a [crate::safety::Version],
}

pub fn show(ui: &mut Ui, specs: &[PanelSpec], cx: &Context<'_>) -> Vec<Action> {
    let mut actions = Vec::new();
    let colors = &cx.theme.colors;
    let bg = color(colors.panel.as_deref().unwrap_or(&colors.background));
    let fg = color(colors.panel_text.as_deref().unwrap_or(&colors.status));

    for (i, spec) in specs.iter().enumerate() {
        // The size and side are part of the id, so a new size from
        // settings.toml takes effect instead of egui's remembered one.
        let id = ("folio-panel", i, spec.side.name(), spec.size.round() as i32);
        let panel = match spec.side {
            Side::Left => egui::Panel::left(id),
            Side::Right => egui::Panel::right(id),
            Side::Bottom => egui::Panel::bottom(id),
        };
        let response = panel
            .resizable(true)
            .default_size(spec.size)
            .frame(egui::Frame::NONE.fill(bg).inner_margin(Margin::same(14)))
            .show(ui, |ui| {
                ui.visuals_mut().override_text_color = Some(fg);
                header(ui, i, spec, cx, &mut actions);
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt(("folio-panel-scroll", i))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match spec.kind {
                        PanelKind::Outline => outline(ui, cx, &mut actions),
                        PanelKind::Text => {
                            for line in cx.text_lines.get(i).into_iter().flatten() {
                                ui.label(line.eval(cx.fields));
                            }
                        }
                        PanelKind::Component => component(ui, spec, cx, &mut actions),
                        PanelKind::Versions => versions(ui, cx, &mut actions),
                    });
            });

        let rect = response.response.rect;
        let size = if spec.side == Side::Bottom { rect.height() } else { rect.width() };
        let dragging = ui.input(|i| i.pointer.any_down());
        if !dragging && (size - spec.size).abs() > 2.0 {
            actions.push(Action::Resize(i, size));
        }
    }
    actions
}

fn header(ui: &mut Ui, i: usize, spec: &PanelSpec, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let title = spec.title.clone().unwrap_or_else(|| match spec.kind {
        PanelKind::Outline => "Outline".into(),
        PanelKind::Text => String::new(),
        PanelKind::Component => component_panel_title(spec, cx).unwrap_or_else(|| "Component".into()),
        PanelKind::Versions => "Versions".into(),
    });
    let label = ui.add(
        egui::Label::new(RichText::new(if title.is_empty() { "···".into() } else { title.to_uppercase() }).small().strong())
            .sense(egui::Sense::click()),
    );
    label.on_hover_text("Right-click to move or close").context_menu(|ui| {
        for side in [Side::Left, Side::Right, Side::Bottom] {
            if side != spec.side && ui.button(format!("Move {}", capitalize(side.name()))).clicked() {
                actions.push(Action::Move(i, side));
            }
        }
        ui.separator();
        if ui.button("Close Panel").clicked() {
            actions.push(Action::Close(i));
        }
    });
}

fn outline(ui: &mut Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let headings: Vec<(usize, u8)> = cx
        .doc
        .paragraphs
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, cx.theme.outline_level(&p.style)?)))
        .collect();
    if headings.is_empty() {
        ui.label(RichText::new("No headings yet").weak());
        return;
    }
    // The heading whose section holds the caret.
    let current = headings.iter().rev().find(|(i, _)| *i <= cx.cursor_para).map(|(i, _)| *i);
    for (para, level) in headings {
        let text: String = cx.doc.paragraphs[para].chars.iter().take(80).collect();
        let text = if text.trim().is_empty() { "(empty heading)".to_owned() } else { text };
        ui.horizontal(|ui| {
            ui.add_space(f32::from(level.saturating_sub(1)) * 12.0);
            if ui.selectable_label(current == Some(para), text).clicked() {
                actions.push(Action::Goto(para));
            }
        });
    }
}

fn component_panel_title(spec: &PanelSpec, cx: &Context<'_>) -> Option<String> {
    let c = &cx.host.components[cx.host.index_of(spec.component.as_deref()?)?];
    let id = spec.id.as_deref()?;
    c.manifest.panels.iter().find(|p| p.id == id).map(|p| p.title.clone())
}

fn component(ui: &mut Ui, spec: &PanelSpec, cx: &Context<'_>, actions: &mut Vec<Action>) {
    let (Some(name), Some(id)) = (spec.component.as_deref(), spec.id.as_deref()) else {
        ui.label(RichText::new("Set `component` and `id` for this panel in settings.toml").weak());
        return;
    };
    let Some(c) = cx.host.index_of(name).map(|i| &cx.host.components[i]) else {
        ui.label(RichText::new(format!("“{name}” isn't installed")).weak());
        return;
    };
    let Some(lines) = c.panels.get(id) else {
        ui.label(RichText::new("Waiting for the component…").weak());
        return;
    };
    for line in lines {
        match &line.command {
            Some(cmd) => {
                if ui.link(&line.text).clicked() {
                    actions.push(Action::Run(Command::Component(name.to_owned(), cmd.clone())));
                }
            }
            None => {
                ui.label(&line.text);
            }
        }
    }
}

fn versions(ui: &mut Ui, cx: &Context<'_>, actions: &mut Vec<Action>) {
    if cx.versions.is_empty() {
        ui.label(RichText::new("Versions appear here each time you save.").weak());
        return;
    }
    for v in cx.versions {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&v.when).small());
            if ui.small_button("Restore").clicked() {
                actions.push(Action::RestoreVersion(v.path.clone()));
            }
        });
        ui.label(RichText::new(format!("{} words · {}", v.words, v.preview)).weak().small());
        ui.add_space(4.0);
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
