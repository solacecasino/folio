//! The menus, built from the keymap so they always show the user's own
//! shortcuts. They're rebuilt whenever keys, the theme or adapters change.
//!
//! On macOS they're the native menu bar. Menu shortcuts are taken by macOS
//! before the window sees the key press, so every item maps to a
//! [`Command`] the app runs itself. On Windows and Linux the same menus are
//! drawn in the window's top strip, in the theme's colors, and shortcuts go
//! through the keymap as usual.
//!
//! Copy, cut and paste are left out on purpose: those keys go straight to the
//! editor.

use crate::keys::{Builtin, Command, Keymap};

/// What goes in the menus besides the built-in commands.
pub struct MenuContent<'a> {
    /// Paragraph styles of the current theme, in order.
    pub styles: Vec<String>,
    /// (adapter name, its commands)
    pub adapters: Vec<(String, Vec<AdapterItem>)>,
    /// WebAssembly plugins' commands, shown in a Plugins menu.
    pub plugins: Vec<crate::plugins::PluginCommand>,
    /// Commands init.lua registered (name, label), shown in a Scripts menu.
    pub scripts: Vec<(String, String)>,
    /// Documents for File → Open Recent, newest first.
    pub recent: Vec<std::path::PathBuf>,
    pub keymap: &'a Keymap,
}

pub struct AdapterItem {
    pub id: String,
    pub label: String,
    /// "file", "edit", "format" or "view" to show it in that menu; other
    /// adapter commands live in the Adapters window.
    pub menu: Option<String>,
}

/// One line in a menu.
enum Entry {
    Item(Command, String),
    Separator,
    /// A nested menu, such as Open Recent.
    Submenu(String, Vec<Entry>),
    About,
    /// Items macOS provides itself (Hide, Minimize, ...).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    System(SystemItem),
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum SystemItem {
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Fullscreen,
}

struct Section {
    key: &'static str,
    title: &'static str,
    entries: Vec<Entry>,
}

/// The menus, the same on every platform except for macOS's own items.
fn sections(content: &MenuContent<'_>) -> Vec<Section> {
    let b = |b: Builtin| {
        let command = Command::Builtin(b);
        let label = command.label();
        Entry::Item(command, label)
    };
    let mac = cfg!(target_os = "macos");

    let mut app = vec![
        Entry::About,
        Entry::Separator,
        b(Builtin::OpenConfigFolder),
        b(Builtin::Adapters),
        b(Builtin::ReloadConfig),
        Entry::Separator,
    ];
    if mac {
        app.extend([
            Entry::System(SystemItem::Hide),
            Entry::System(SystemItem::HideOthers),
            Entry::System(SystemItem::ShowAll),
            Entry::Separator,
        ]);
    }
    app.push(b(Builtin::Quit));

    let mut format = vec![b(Builtin::Bold), b(Builtin::Italic), b(Builtin::Underline), Entry::Separator];
    format.extend(content.styles.iter().map(|s| Entry::Item(Command::Style(s.clone()), s.clone())));

    let mut all = vec![
        Section { key: "app", title: "Folio", entries: app },
        Section {
            key: "file",
            title: "File",
            entries: vec![
                b(Builtin::New),
                b(Builtin::Open),
                Entry::Submenu(
                    "Open Recent".into(),
                    content
                        .recent
                        .iter()
                        .map(|p| {
                            let command = Command::OpenFile(p.clone());
                            let label = command.label();
                            Entry::Item(command, label)
                        })
                        .collect(),
                ),
                Entry::Separator,
                b(Builtin::Save),
                b(Builtin::SaveAs),
                b(Builtin::Versions),
            ],
        },
        Section {
            key: "edit",
            title: "Edit",
            entries: vec![
                b(Builtin::Undo),
                b(Builtin::Redo),
                Entry::Separator,
                b(Builtin::SelectAll),
                Entry::Separator,
                b(Builtin::Find),
                b(Builtin::FindNext),
                b(Builtin::FindPrevious),
                b(Builtin::Replace),
            ],
        },
        Section { key: "format", title: "Format", entries: format },
        Section {
            key: "view",
            title: "View",
            entries: vec![
                b(Builtin::NextTheme),
                Entry::Separator,
                b(Builtin::ZoomIn),
                b(Builtin::ZoomOut),
                b(Builtin::ZoomReset),
                Entry::Separator,
                b(Builtin::ToggleStatus),
                b(Builtin::TogglePanels),
            ],
        },
        Section {
            key: "help",
            title: "Help",
            entries: vec![
                b(Builtin::Guide),
                b(Builtin::Help),
                Entry::Separator,
                b(Builtin::ReportProblem),
                b(Builtin::Log),
            ],
        },
    ];

    // Adapter commands that asked to live in one of Folio's own menus.
    for section in &mut all {
        let placed: Vec<Entry> = content
            .adapters
            .iter()
            .flat_map(|(name, items)| {
                items
                    .iter()
                    .filter(|i| i.menu.as_deref() == Some(section.key))
                    .map(|i| Entry::Item(Command::Adapter(name.clone(), i.id.clone()), i.label.clone()))
            })
            .collect();
        if !placed.is_empty() {
            section.entries.push(Entry::Separator);
            section.entries.extend(placed);
        }
    }

    if !content.plugins.is_empty() {
        all.push(Section {
            key: "plugins",
            title: "Plugins",
            entries: content
                .plugins
                .iter()
                .map(|c| Entry::Item(Command::Plugin(c.plugin.clone(), c.id.clone()), c.label.clone()))
                .collect(),
        });
    }

    if !content.scripts.is_empty() {
        all.push(Section {
            key: "scripts",
            title: "Scripts",
            entries: content
                .scripts
                .iter()
                .map(|(name, label)| Entry::Item(Command::Lua(name.clone()), label.clone()))
                .collect(),
        });
    }

    if mac {
        all.push(Section {
            key: "window",
            title: "Window",
            entries: vec![Entry::System(SystemItem::Minimize), Entry::System(SystemItem::Fullscreen)],
        });
    }
    all
}

#[cfg(target_os = "macos")]
pub use native::MenuBar;

#[cfg(not(target_os = "macos"))]
pub use drawn::MenuBar;

#[cfg(target_os = "macos")]
mod native {
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, Sender, channel};

    use muda::{
        AboutMetadataBuilder, Icon, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::Accelerator,
    };

    use super::{Entry, MenuContent, SystemItem, sections};
    use crate::keys::Command;
    use crate::theme::Theme;

    pub struct MenuBar {
        // Dropping the menu removes it from the menu bar, so keep it alive.
        _menu: Menu,
        /// Menu item ids are indexes into this list.
        commands: Vec<Command>,
        events: Receiver<String>,
    }

    // muda allows one event handler per process, so it forwards ids to
    // whichever menu is current through this sender.
    static EVENTS: Mutex<Option<Sender<String>>> = Mutex::new(None);

    impl MenuBar {
        pub fn install(ctx: &egui::Context, icon: &egui::IconData, content: &MenuContent<'_>) -> Self {
            let mut commands: Vec<Command> = Vec::new();
            let menu = Menu::new();
            let mut window_menu = None;

            for section in sections(content) {
                let sub = Submenu::new(section.title, true);
                fill(&sub, section.entries, content, icon, &mut commands);
                menu.append(&sub).expect("menu builds");
                if section.key == "window" {
                    window_menu = Some(sub);
                }
            }
            menu.init_for_nsapp();
            if let Some(w) = &window_menu {
                w.set_as_windows_menu_for_nsapp();
            }

            let (tx, events) = channel();
            let mut slot = EVENTS.lock().expect("menu lock");
            if slot.is_none() {
                let ctx = ctx.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    if let Some(tx) = EVENTS.lock().expect("menu lock").as_ref() {
                        let _ = tx.send(event.id.0);
                    }
                    // The app may be idle; wake it so the command runs right away.
                    ctx.request_repaint();
                }));
            }
            *slot = Some(tx);

            Self {
                _menu: menu,
                commands,
                events,
            }
        }

        pub fn poll(&mut self) -> Vec<Command> {
            self.events
                .try_iter()
                .filter_map(|id| id.strip_prefix('m')?.parse::<usize>().ok())
                .filter_map(|i| self.commands.get(i).cloned())
                .collect()
        }

        /// The macOS menu bar draws itself.
        pub fn show(&mut self, _ui: &mut egui::Ui, _rect: egui::Rect, _theme: &Theme) {}
    }

    /// Adds entries to a menu; nested menus recurse.
    fn fill(sub: &Submenu, entries: Vec<Entry>, content: &MenuContent<'_>, icon: &egui::IconData, commands: &mut Vec<Command>) {
        for entry in entries {
            let item: Box<dyn IsMenuItem> = match entry {
                Entry::Item(command, label) => {
                    let accel = content
                        .keymap
                        .chord_for(&command)
                        .and_then(|c| c.accelerator().parse::<Accelerator>().ok());
                    let id = format!("m{}", commands.len());
                    commands.push(command);
                    Box::new(MenuItem::with_id(id, label, true, accel))
                }
                Entry::Submenu(title, children) => {
                    let nested = Submenu::new(title, !children.is_empty());
                    fill(&nested, children, content, icon, commands);
                    Box::new(nested)
                }
                Entry::Separator => Box::new(PredefinedMenuItem::separator()),
                Entry::About => Box::new(PredefinedMenuItem::about(Some("About Folio"), Some(about(icon)))),
                Entry::System(s) => Box::new(match s {
                    SystemItem::Hide => PredefinedMenuItem::hide(None),
                    SystemItem::HideOthers => PredefinedMenuItem::hide_others(None),
                    SystemItem::ShowAll => PredefinedMenuItem::show_all(None),
                    SystemItem::Minimize => PredefinedMenuItem::minimize(None),
                    SystemItem::Fullscreen => PredefinedMenuItem::fullscreen(None),
                }),
            };
            sub.append(item.as_ref()).expect("menu builds");
        }
    }

    fn about(icon: &egui::IconData) -> muda::AboutMetadata {
        AboutMetadataBuilder::new()
            .name(Some(crate::plugins::EDITION))
            .version(Some(crate::plugins::version()))
            .comments(Some(crate::plugins::about_text()))
            .icon(Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height).ok())
            .build()
    }
}

#[cfg(not(target_os = "macos"))]
mod drawn {
    use egui::{self, RichText};

    use super::{Entry, MenuContent, sections};
    use crate::keys::Command;
    use crate::theme::{Theme, color};

    /// An entry with its shortcut text.
    struct Shown {
        entry: Entry,
        keys: String,
        children: Vec<Shown>,
    }

    fn shown(entries: Vec<Entry>, content: &MenuContent<'_>) -> Vec<Shown> {
        entries
            .into_iter()
            .map(|e| {
                let keys = match &e {
                    Entry::Item(c, _) => content.keymap.chord_for(c).map(|c| c.accelerator()),
                    _ => None,
                };
                let (entry, children) = match e {
                    Entry::Submenu(title, children) => (Entry::Submenu(title, Vec::new()), shown(children, content)),
                    other => (other, Vec::new()),
                };
                Shown {
                    entry,
                    keys: keys.unwrap_or_default(),
                    children,
                }
            })
            .collect()
    }

    pub struct MenuBar {
        menus: Vec<(&'static str, Vec<Shown>)>,
        clicked: Vec<Command>,
        about_open: bool,
        icon: Option<egui::TextureHandle>,
        icon_data: egui::IconData,
    }

    impl MenuBar {
        pub fn install(_ctx: &egui::Context, icon: &egui::IconData, content: &MenuContent<'_>) -> Self {
            let menus = sections(content).into_iter().map(|s| (s.title, shown(s.entries, content))).collect();
            Self {
                menus,
                clicked: Vec::new(),
                about_open: false,
                icon: None,
                icon_data: icon.clone(),
            }
        }

        pub fn poll(&mut self) -> Vec<Command> {
            std::mem::take(&mut self.clicked)
        }

        /// Draws the menus in the window's top strip.
        pub fn show(&mut self, ui: &mut egui::Ui, rect: egui::Rect, theme: &Theme) {
            let colors = &theme.colors;
            let bg = color(&colors.background);
            let fg = color(&colors.status);
            ui.painter().rect_filled(rect, 0.0, bg);
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(6.0, 0.0))));
            bar.visuals_mut().override_text_color = Some(fg);
            // Hovered and open menu titles get a faint tint of the text color,
            // so the (light) title stays readable on any theme.
            let w = &mut bar.visuals_mut().widgets;
            w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            for state in [&mut w.hovered, &mut w.active, &mut w.open] {
                state.weak_bg_fill = fg.gamma_multiply(0.18);
                state.bg_fill = fg.gamma_multiply(0.18);
            }

            egui::MenuBar::new().ui(&mut bar, |ui| {
                for (title, entries) in &self.menus {
                    ui.menu_button(RichText::new(*title).color(fg), |ui| {
                        draw(ui, entries, &mut self.clicked, &mut self.about_open);
                    });
                }
            });

            if self.about_open {
                let icon = self.icon.get_or_insert_with(|| {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [self.icon_data.width as usize, self.icon_data.height as usize],
                        &self.icon_data.rgba,
                    );
                    ui.ctx().load_texture("folio-icon", image, egui::TextureOptions::LINEAR)
                });
                egui::Window::new("About Folio")
                    .open(&mut self.about_open)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .show(ui.ctx(), |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add(egui::Image::new(&*icon).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                            ui.heading(crate::plugins::EDITION);
                            ui.label(format!("Version {}", crate::plugins::version()));
                            ui.label(crate::plugins::about_text());
                        });
                    });
            }
        }
    }

    fn draw(ui: &mut egui::Ui, entries: &[Shown], clicked: &mut Vec<Command>, about_open: &mut bool) {
        ui.set_min_width(220.0);
        for s in entries {
            match &s.entry {
                Entry::Item(command, label) => {
                    if ui.add(egui::Button::new(label.as_str()).shortcut_text(s.keys.as_str())).clicked() {
                        clicked.push(command.clone());
                    }
                }
                Entry::Submenu(title, _) => {
                    ui.add_enabled_ui(!s.children.is_empty(), |ui| {
                        ui.menu_button(title.as_str(), |ui| draw(ui, &s.children, clicked, about_open));
                    });
                }
                Entry::Separator => {
                    ui.separator();
                }
                Entry::About => {
                    if ui.button(format!("About {}", crate::plugins::EDITION)).clicked() {
                        *about_open = true;
                    }
                }
                Entry::System(_) => {}
            }
        }
    }
}
