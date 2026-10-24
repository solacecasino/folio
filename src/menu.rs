//! The native macOS menu bar, built from the keymap so it always shows the
//! user's own shortcuts. It's rebuilt whenever keys, the theme or adapters
//! change.
//!
//! Menu shortcuts are taken by macOS before the window sees the key press, so
//! every item maps to a [`Command`] the app runs itself. Copy, cut and paste
//! are left out on purpose: those keys go straight to the editor.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui;
use muda::{
    AboutMetadataBuilder, Icon, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::Accelerator,
};

use crate::keys::{Builtin, Command, Keymap};

/// What goes in the menus besides the built-in commands.
pub struct MenuContent<'a> {
    /// Paragraph styles of the current theme, in order.
    pub styles: Vec<String>,
    /// (adapter name, its commands)
    pub adapters: Vec<(String, Vec<AdapterItem>)>,
    pub keymap: &'a Keymap,
}

pub struct AdapterItem {
    pub id: String,
    pub label: String,
    /// "file", "edit", "format" or "view" to place it in that menu.
    pub menu: Option<String>,
}

pub struct MenuBar {
    // Dropping the menu removes it from the menu bar, so keep it alive.
    _menu: Menu,
    /// Menu item ids are indexes into this list.
    commands: Vec<Command>,
    events: Receiver<String>,
}

// muda allows one event handler per process, so it forwards ids to whichever
// menu is current through this sender.
static EVENTS: Mutex<Option<Sender<String>>> = Mutex::new(None);

impl MenuBar {
    pub fn install(ctx: &egui::Context, icon: &egui::IconData, content: &MenuContent<'_>) -> Self {
        let mut commands: Vec<Command> = Vec::new();
        let mut item = |command: Command, label: &str| {
            let accel = content
                .keymap
                .chord_for(&command)
                .and_then(|c| c.accelerator().parse::<Accelerator>().ok());
            let id = format!("m{}", commands.len());
            commands.push(command);
            MenuItem::with_id(id, label, true, accel)
        };
        let builtin = |item: &mut dyn FnMut(Command, &str) -> MenuItem, b: Builtin| {
            let command = Command::Builtin(b);
            let label = command.label();
            item(command, &label)
        };
        let sep = PredefinedMenuItem::separator;
        // Adapter commands that asked to live in one of Folio's own menus.
        let placed = |menu: &str| -> Vec<(Command, String)> {
            content
                .adapters
                .iter()
                .flat_map(|(name, items)| {
                    items
                        .iter()
                        .filter(move |i| i.menu.as_deref() == Some(menu))
                        .map(move |i| (Command::Adapter(name.clone(), i.id.clone()), i.label.clone()))
                })
                .collect()
        };

        let app = Submenu::with_items(
            "Folio",
            true,
            &[
                &PredefinedMenuItem::about(Some("About Folio"), Some(about(icon))),
                &sep(),
                &builtin(&mut item, Builtin::OpenConfigFolder),
                &builtin(&mut item, Builtin::OpenAdaptersFolder),
                &builtin(&mut item, Builtin::ReloadConfig),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &builtin(&mut item, Builtin::Quit),
            ],
        );
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &builtin(&mut item, Builtin::New),
                &builtin(&mut item, Builtin::Open),
                &sep(),
                &builtin(&mut item, Builtin::Save),
                &builtin(&mut item, Builtin::SaveAs),
                &builtin(&mut item, Builtin::Versions),
            ],
        );
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[
                &builtin(&mut item, Builtin::Undo),
                &builtin(&mut item, Builtin::Redo),
                &sep(),
                &builtin(&mut item, Builtin::SelectAll),
                &sep(),
                &builtin(&mut item, Builtin::Find),
                &builtin(&mut item, Builtin::FindNext),
                &builtin(&mut item, Builtin::FindPrevious),
                &builtin(&mut item, Builtin::Replace),
            ],
        );
        let format = Submenu::with_items(
            "Format",
            true,
            &[
                &builtin(&mut item, Builtin::Bold),
                &builtin(&mut item, Builtin::Italic),
                &builtin(&mut item, Builtin::Underline),
                &sep(),
            ],
        )
        .expect("menu builds");
        for style in &content.styles {
            format.append(&item(Command::Style(style.clone()), style)).expect("menu builds");
        }
        let view = Submenu::with_items(
            "View",
            true,
            &[
                &builtin(&mut item, Builtin::NextTheme),
                &sep(),
                &builtin(&mut item, Builtin::ZoomIn),
                &builtin(&mut item, Builtin::ZoomOut),
                &builtin(&mut item, Builtin::ZoomReset),
                &sep(),
                &builtin(&mut item, Builtin::ToggleStatus),
                &builtin(&mut item, Builtin::TogglePanels),
                &builtin(&mut item, Builtin::Help),
            ],
        );

        // Bare by default: this menu only exists once an adapter is installed.
        let own: Vec<(&String, Vec<&AdapterItem>)> = content
            .adapters
            .iter()
            .map(|(name, items)| (name, items.iter().filter(|i| i.menu.is_none()).collect::<Vec<_>>()))
            .filter(|(_, items)| !items.is_empty())
            .collect();
        let adapters = (!own.is_empty()).then(|| {
            let list = Submenu::new("Adapters", true);
            for (name, items) in &own {
                let sub = Submenu::new(*name, true);
                for i in items {
                    sub.append(&item(Command::Adapter((*name).clone(), i.id.clone()), &i.label))
                        .expect("menu builds");
                }
                list.append(&sub).expect("menu builds");
            }
            list
        });

        let window = Submenu::with_items(
            "Window",
            true,
            &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::fullscreen(None)],
        )
        .expect("menu builds");

        let menu = Menu::new();
        let app = app.expect("menu builds");
        let standard = [
            ("file", file.expect("menu builds")),
            ("edit", edit.expect("menu builds")),
            ("format", format),
            ("view", view.expect("menu builds")),
        ];
        menu.append(&app).expect("menu builds");
        for (key, sub) in &standard {
            let extra = placed(key);
            if !extra.is_empty() {
                sub.append(&sep()).expect("menu builds");
                for (command, label) in extra {
                    sub.append(&item(command, &label)).expect("menu builds");
                }
            }
            menu.append(sub).expect("menu builds");
        }
        if let Some(adapters) = &adapters {
            menu.append(adapters).expect("menu builds");
        }
        menu.append(&window).expect("menu builds");
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

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

    pub fn poll(&self) -> Vec<Command> {
        self.events
            .try_iter()
            .filter_map(|id| id.strip_prefix('m')?.parse::<usize>().ok())
            .filter_map(|i| self.commands.get(i).cloned())
            .collect()
    }
}

fn about(icon: &egui::IconData) -> muda::AboutMetadata {
    AboutMetadataBuilder::new()
        .name(Some("Folio"))
        .version(Some(env!("CARGO_PKG_VERSION")))
        .comments(Some("Bare when installed, insanely customizable."))
        .icon(Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height).ok())
        .build()
}
