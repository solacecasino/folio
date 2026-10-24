//! The Adapters window: what's installed, what each adapter may do, its
//! commands, and adding or removing adapters. Folio ships with none.

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, ViewportBuilder, ViewportId};

use crate::adapters::{self, Host};
use crate::keys::{Command, Keymap};
use crate::theme::{Theme, color};

pub const GET_ADAPTERS_URL: &str = "https://github.com/solacecasino/folio-adapters/releases";

pub enum Action {
    AddZip,
    AddFolder,
    GetAdapters,
    ShowFolder,
    Run(Command),
    Remove(usize),
    Close,
}

/// Shows the window (a real second macOS window) and returns what was clicked.
pub fn show(ctx: &egui::Context, host: &Host, theme: &Theme, keymap: &Keymap) -> Vec<Action> {
    let mut actions = Vec::new();
    let colors = &theme.colors;
    let bg = color(colors.panel.as_deref().unwrap_or(&colors.background));
    let fg = color(colors.panel_text.as_deref().unwrap_or(&colors.status));
    let strong = color(&colors.page);
    let card = bg.lerp_to_gamma(fg, 0.07);

    ctx.show_viewport_immediate(
        ViewportId::from_hash_of("folio-adapters"),
        ViewportBuilder::default()
            .with_title("Adapters")
            .with_inner_size([560.0, 640.0])
            .with_min_inner_size([420.0, 320.0]),
        |ui, _class| {
            if ui.input(|i| i.viewport().close_requested()) {
                actions.push(Action::Close);
            }
            ui.painter().rect_filled(ui.max_rect(), 0.0, bg);
            ui.visuals_mut().override_text_color = Some(fg);

            egui::Frame::NONE.inner_margin(Margin::same(20)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Adapters").size(22.0).strong().color(strong_text(strong, fg, bg)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Show Folder").clicked() {
                            actions.push(Action::ShowFolder);
                        }
                        if ui.button("Get Adapters").on_hover_text(GET_ADAPTERS_URL).clicked() {
                            actions.push(Action::GetAdapters);
                        }
                        if ui.button(RichText::new("Add Adapter…").strong()).clicked() {
                            actions.push(Action::AddZip);
                        }
                    });
                });
                ui.label(
                    RichText::new("Folio ships bare. Add only what you want; each adapter asks before it first runs.")
                        .small(),
                );
                ui.add_space(14.0);

                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let installed: Vec<_> = host.installed().collect();
                    if installed.is_empty() {
                        empty_state(ui, &mut actions);
                    }
                    for (i, a) in installed {
                        adapter_card(ui, i, a, card, fg, keymap, &mut actions);
                        ui.add_space(10.0);
                    }
                    ui.add_space(6.0);
                    if ui.link("Add an adapter from a folder instead…").clicked() {
                        actions.push(Action::AddFolder);
                    }
                });
            });
        },
    );
    actions
}

/// Titles in the page color unless that would be invisible on the panel.
fn strong_text(page: Color32, fg: Color32, bg: Color32) -> Color32 {
    let luma = |c: Color32| {
        let [r, g, b, _] = c.to_array();
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)
    };
    if (luma(page) - luma(bg)).abs() > 80.0 { page } else { fg }
}

fn empty_state(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("No adapters yet").size(17.0).strong());
        ui.add_space(4.0);
        ui.label("Folio is bare until you add one. Adapters add things like PDF export or live co-editing.");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
            if ui.button("Get Adapters").clicked() {
                actions.push(Action::GetAdapters);
            }
            if ui.button("Add Adapter…").clicked() {
                actions.push(Action::AddZip);
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Download an adapter's .zip, then choose Add Adapter… and pick it.").small());
    });
    ui.add_space(30.0);
}

fn adapter_card(
    ui: &mut egui::Ui,
    i: usize,
    a: &adapters::Adapter,
    fill: Color32,
    fg: Color32,
    keymap: &Keymap,
    actions: &mut Vec<Action>,
) {
    let m = &a.manifest;
    egui::Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, fg.gamma_multiply(0.15)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&m.name).size(16.0).strong());
                ui.label(RichText::new(&m.version).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Remove").clicked() {
                        actions.push(Action::Remove(i));
                    }
                    let (dot, state) = if a.is_running() {
                        (Color32::from_rgb(80, 190, 110), "Running")
                    } else if a.is_blocked() {
                        (Color32::from_rgb(220, 90, 80), "Not allowed")
                    } else {
                        (fg.gamma_multiply(0.5), "Stopped")
                    };
                    ui.label(RichText::new(state).small());
                    ui.label(RichText::new("●").small().color(dot));
                });
            });

            let by: Vec<&str> = [m.author.as_str(), m.license.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
            if !by.is_empty() {
                ui.label(RichText::new(by.join(" · ")).small());
            }
            if !m.description.is_empty() {
                ui.add_space(4.0);
                ui.label(&m.description);
            }

            ui.add_space(6.0);
            let can = if m.permissions.is_empty() {
                "Doesn't ask for any permissions".to_owned()
            } else {
                let list: Vec<String> = m.permissions.iter().map(|p| adapters::describe(p)).collect();
                format!("Can {}", list.join(", "))
            };
            ui.label(RichText::new(can).small());

            if !m.commands.is_empty() {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for cmd in &m.commands {
                        let command = Command::Adapter(m.name.clone(), cmd.id.clone());
                        let keys = keymap.chord_for(&command).map(|c| c.accelerator()).unwrap_or_default();
                        let button = ui.button(&cmd.label);
                        let button = if keys.is_empty() { button } else { button.on_hover_text(keys) };
                        if button.clicked() {
                            actions.push(Action::Run(command));
                        }
                    }
                });
            }
        });
}
