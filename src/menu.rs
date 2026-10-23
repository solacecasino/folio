//! The native macOS menu bar.
//!
//! Menu shortcuts are taken by macOS before the window sees the key press, so
//! every item here maps to a [`Command`] the app runs itself. Copy, cut and
//! paste are left out on purpose: those keys go straight to the editor.

use std::sync::mpsc::{Receiver, channel};

use eframe::egui;
use muda::{
    AboutMetadataBuilder, Icon, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::Accelerator,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Quit,
    Undo,
    Redo,
    SelectAll,
    Bold,
    Italic,
    Underline,
    NextTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleStatus,
    Help,
    /// A component's command: (component, command) indexes.
    Component(usize, usize),
}

/// A component's commands, as they appear in the Components menu.
pub struct ComponentMenu {
    pub name: String,
    /// (label, shortcut) for each command, in order.
    pub items: Vec<(String, Option<String>)>,
}

const ITEMS: &[(Command, &str, &str)] = &[
    (Command::New, "New", "Cmd+N"),
    (Command::Open, "Open…", "Cmd+O"),
    (Command::Save, "Save", "Cmd+S"),
    (Command::SaveAs, "Save As…", "Cmd+Shift+S"),
    (Command::Quit, "Quit Folio", "Cmd+Q"),
    (Command::Undo, "Undo", "Cmd+Z"),
    (Command::Redo, "Redo", "Cmd+Shift+Z"),
    (Command::SelectAll, "Select All", "Cmd+A"),
    (Command::Bold, "Bold", "Cmd+B"),
    (Command::Italic, "Italic", "Cmd+I"),
    (Command::Underline, "Underline", "Cmd+U"),
    (Command::NextTheme, "Next Theme", "Cmd+T"),
    (Command::ZoomIn, "Zoom In", "Cmd+="),
    (Command::ZoomOut, "Zoom Out", "Cmd+-"),
    // Cmd+0 belongs to paragraph styles, so this one has no shortcut.
    (Command::ZoomReset, "Actual Size", ""),
    (Command::ToggleStatus, "Show Status Line", "Cmd+."),
    (Command::Help, "Keyboard Shortcuts", "Cmd+/"),
];

pub struct MenuBar {
    // Dropping the menu removes it from the menu bar, so keep it alive.
    _menu: Menu,
    events: Receiver<Command>,
}

impl MenuBar {
    pub fn install(ctx: &egui::Context, icon: &egui::IconData, components: &[ComponentMenu]) -> Self {
        let item = |cmd: Command| {
            let (_, label, keys) = ITEMS.iter().find(|(c, ..)| *c == cmd).expect("command listed");
            let accel: Option<Accelerator> = keys.parse().ok();
            MenuItem::with_id(format!("{cmd:?}"), label, true, accel)
        };
        let sep = PredefinedMenuItem::separator;

        let app = Submenu::with_items(
            "Folio",
            true,
            &[
                &PredefinedMenuItem::about(Some("About Folio"), Some(about(icon))),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &item(Command::Quit),
            ],
        );
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &item(Command::New),
                &item(Command::Open),
                &sep(),
                &item(Command::Save),
                &item(Command::SaveAs),
            ],
        );
        let edit = Submenu::with_items(
            "Edit",
            true,
            &[
                &item(Command::Undo),
                &item(Command::Redo),
                &sep(),
                &item(Command::SelectAll),
            ],
        );
        let format = Submenu::with_items(
            "Format",
            true,
            &[&item(Command::Bold), &item(Command::Italic), &item(Command::Underline)],
        );
        let view = Submenu::with_items(
            "View",
            true,
            &[
                &item(Command::NextTheme),
                &sep(),
                &item(Command::ZoomIn),
                &item(Command::ZoomOut),
                &item(Command::ZoomReset),
                &sep(),
                &item(Command::ToggleStatus),
                &item(Command::Help),
            ],
        );
        let window = Submenu::with_items(
            "Window",
            true,
            &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::fullscreen(None)],
        )
        .expect("menu builds");

        let menu = Menu::new();
        for sub in [app, file, edit, format, view] {
            menu.append(&sub.expect("menu builds")).expect("menu builds");
        }
        // Bare by default: the menu only exists once a component is installed.
        if !components.is_empty() {
            let list = Submenu::new("Components", true);
            for (i, c) in components.iter().enumerate() {
                let sub = Submenu::new(&c.name, true);
                for (j, (label, keys)) in c.items.iter().enumerate() {
                    let accel = keys.as_deref().and_then(|k| k.parse::<Accelerator>().ok());
                    sub.append(&MenuItem::with_id(format!("component:{i}:{j}"), label, true, accel))
                        .expect("menu builds");
                }
                list.append(&sub).expect("menu builds");
            }
            menu.append(&list).expect("menu builds");
        }
        menu.append(&window).expect("menu builds");
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();

        let (tx, events) = channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id.0.as_str();
            let cmd = match id.strip_prefix("component:").and_then(|rest| rest.split_once(':')) {
                Some((i, j)) => i.parse().ok().zip(j.parse().ok()).map(|(i, j)| Command::Component(i, j)),
                None => ITEMS.iter().find(|(c, ..)| format!("{c:?}") == id).map(|(c, ..)| *c),
            };
            if let Some(cmd) = cmd {
                let _ = tx.send(cmd);
                // The app may be idle; wake it so the command runs right away.
                ctx.request_repaint();
            }
        }));

        Self { _menu: menu, events }
    }

    pub fn poll(&self) -> Vec<Command> {
        self.events.try_iter().collect()
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
