//! WebAssembly plugins: sandboxed extensions that run inside text9000 (see
//! `crates/folio_plugin`). This module connects them to the editor: it
//! publishes the document as it changes, sends events, and turns what
//! plugins send back into effects the app applies.
//!
//! Plugins need the `wasi-plugins` feature (on by default). Without it,
//! as in folio-lite, the same API exists and does nothing, so the rest of
//! the app needs no feature checks.

/// What the app should do because of a plugin.
// folio-lite never produces any.
#[cfg_attr(not(feature = "wasi-plugins"), allow(dead_code))]
pub enum Effect {
    /// Insert text at the caret, made against this document revision.
    Insert { plugin: String, revision: u64, text: String },
    /// Replace a range, made against this document revision.
    Replace {
        plugin: String,
        revision: u64,
        start: crate::doc::Pos,
        end: crate::doc::Pos,
        text: String,
    },
    /// A short message to show.
    Notify(String),
    /// A plugin that couldn't load (shown until fixed).
    Problem(String),
    /// Plugins and their commands changed; rebuild menus.
    CommandsChanged,
}

/// One line of a plugin's panel; a line with a command is a link.
pub struct PanelLine {
    pub text: String,
    pub command: Option<String>,
}

/// What `plugin.toml` says a plugin is, for asking before installing it.
#[cfg_attr(not(feature = "wasi-plugins"), allow(dead_code))]
pub struct Described {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub can: Vec<String>,
}

/// Where plugins are installed.
#[cfg_attr(not(feature = "wasi-plugins"), allow(dead_code))]
pub fn plugins_dir() -> std::path::PathBuf {
    crate::settings::config_dir().join("plugins")
}

/// One plugin command, for menus and key bindings.
pub struct PluginCommand {
    pub plugin: String,
    pub id: String,
    pub label: String,
}

/// "Folio", or "text9000 Lite" for builds without plugins.
pub const EDITION: &str = if cfg!(feature = "wasi-plugins") { "text9000" } else { "text9000 Lite" };

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The About window's text: the motto, the edition, and where the config folder is.
pub fn about_text() -> String {
    let plugins = if cfg!(feature = "wasi-plugins") {
        "WebAssembly plugins (preview): on"
    } else {
        "text9000 Lite: no plugins; adapters only"
    };
    format!(
        "Bare when installed, insanely customizable.\n\n{plugins}\nConfig folder: {}",
        crate::settings::config_dir().display()
    )
}

#[cfg(feature = "wasi-plugins")]
pub use enabled::Plugins;

#[cfg(not(feature = "wasi-plugins"))]
pub use disabled::Plugins;

#[cfg(feature = "wasi-plugins")]
mod enabled {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use folio_plugin::{
        DocumentSnapshot, Edit, Event, PluginError, PluginOutput, PluginRuntime, Position, RuntimeConfig, Selection,
    };

    use std::collections::{BTreeMap, HashMap};

    use super::{Described, Effect, PanelLine, PluginCommand};
    use crate::doc::Pos;
    use crate::editor::Editor;

    pub struct Plugins {
        /// None if the runtime couldn't start; the problem was reported.
        runtime: Option<PluginRuntime>,
        ctx: egui::Context,
        dir: PathBuf,
        /// Status line text by plugin.
        statuses: BTreeMap<String, String>,
        /// Panel contents by (plugin, panel id).
        panels: HashMap<(String, String), Vec<PanelLine>>,
        startup_problem: Option<String>,
        /// (revision, cursor, anchor) last published, to send only changes.
        sent: Option<(u64, Pos, Pos)>,
        path: Option<PathBuf>,
    }

    impl Plugins {
        pub fn start(ctx: &egui::Context) -> Self {
            Self::start_in(ctx, super::plugins_dir())
        }

        pub fn start_in(ctx: &egui::Context, plugins_dir: PathBuf) -> Self {
            let waker = ctx.clone();
            let config = RuntimeConfig {
                plugins_dir: plugins_dir.clone(),
                waker: Some(Arc::new(move || waker.request_repaint())),
                ..Default::default()
            };
            let (runtime, startup_problem) = match PluginRuntime::start(config) {
                Ok(runtime) => (Some(runtime), None),
                Err(e) => (None, Some(format!("Plugins are off: {e}"))),
            };
            Self {
                runtime,
                ctx: ctx.clone(),
                dir: plugins_dir,
                statuses: BTreeMap::new(),
                panels: HashMap::new(),
                startup_problem,
                sent: None,
                path: None,
            }
        }

        /// Stops every plugin and loads them again, e.g. after installing one.
        pub fn reload(&mut self) {
            let (ctx, dir) = (self.ctx.clone(), self.dir.clone());
            // Dropping the old runtime stops its plugins first.
            self.runtime = None;
            *self = Self::start_in(&ctx, dir);
        }

        /// Reads a plugin folder's manifest, for asking before installing it.
        pub fn describe(dir: &Path) -> Result<Described, String> {
            let m = folio_plugin::Manifest::load(dir).map_err(|e| e.to_string())?;
            if !dir.join(&m.wasm).is_file() {
                return Err(format!("{} is missing {}", m.name, m.wasm.display()));
            }
            Ok(Described {
                can: m.capabilities.describe(),
                name: m.name,
                version: m.version,
                author: m.author,
                description: m.description,
            })
        }

        /// Copies a plugin folder into the plugins folder, replacing an
        /// installed plugin with the same name (the old one goes to the
        /// trash), then reloads plugins.
        pub fn install(&mut self, dir: &Path) -> Result<(), String> {
            let name = folio_plugin::Manifest::load(dir).map_err(|e| e.to_string())?.name;
            std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
            let (installed, _) = folio_plugin::manager::discover(&self.dir);
            for p in installed.iter().filter(|p| p.manifest.name == name) {
                // Loose .wasm plugins live in the plugins folder itself.
                let old = if p.dir == self.dir { p.wasm.clone() } else { p.dir.clone() };
                crate::platform::move_to_trash(&old)?;
            }
            let base: String = name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '-' })
                .collect();
            let mut dest = self.dir.join(&base);
            let mut n = 2;
            while dest.exists() {
                dest = self.dir.join(format!("{base}-{n}"));
                n += 1;
            }
            crate::platform::copy_dir(dir, &dest).map_err(|e| e.to_string())?;
            self.reload();
            Ok(())
        }

        /// What plugins show in the status line.
        pub fn status_items(&self) -> Vec<String> {
            self.statuses.values().cloned().collect()
        }

        pub fn panel(&self, plugin: &str, id: &str) -> Option<&[PanelLine]> {
            self.panels.get(&(plugin.to_owned(), id.to_owned())).map(Vec::as_slice)
        }

        pub fn panel_title(&self, plugin: &str, id: &str) -> Option<String> {
            let p = self.runtime.as_ref()?.plugins().iter().find(|p| p.name == plugin)?;
            p.panels.iter().find(|panel| panel.id == id).map(|panel| panel.title.clone())
        }

        pub fn is_loaded(&self, plugin: &str) -> bool {
            self.runtime.as_ref().is_some_and(|r| r.plugins().iter().any(|p| p.name == plugin))
        }

        /// Loaded plugins as "Name version".
        pub fn loaded(&self) -> Vec<String> {
            self.runtime
                .as_ref()
                .map(|r| r.plugins().iter().map(|p| format!("{} {}", p.name, p.version)).collect())
                .unwrap_or_default()
        }

        /// Publishes the document if it or the selection changed, sends the
        /// matching events, and collects what plugins sent back. Cheap when
        /// nothing changed; never waits.
        pub fn update(&mut self, editor: &Editor, path: Option<&Path>, theme: &str) -> Vec<Effect> {
            let mut effects: Vec<Effect> = self.startup_problem.take().map(Effect::Problem).into_iter().collect();
            let Some(runtime) = &mut self.runtime else { return effects };

            let state = (editor.revision, editor.cursor(), editor.anchor());
            if self.sent != Some(state) || self.path.as_deref() != path {
                let changed = self.sent.is_none_or(|(rev, ..)| rev != state.0);
                runtime.set_document(snapshot(editor, path, theme));
                if self.sent.is_some() {
                    runtime.dispatch(if changed { Event::Changed } else { Event::SelectionChanged });
                }
                self.sent = Some(state);
                self.path = path.map(Path::to_path_buf);
            }

            for output in runtime.poll() {
                effects.push(match output {
                    PluginOutput::Loaded(_) => Effect::CommandsChanged,
                    PluginOutput::Edit {
                        plugin,
                        revision,
                        edit: Edit::InsertText(text),
                    } => Effect::Insert { plugin, revision, text },
                    PluginOutput::Edit {
                        plugin,
                        revision,
                        edit: Edit::ReplaceRange { start, end, text },
                    } => Effect::Replace {
                        plugin,
                        revision,
                        start: Pos::new(start.paragraph as usize, start.character as usize),
                        end: Pos::new(end.paragraph as usize, end.character as usize),
                        text,
                    },
                    PluginOutput::Status { plugin, text } => {
                        match text {
                            Some(t) => self.statuses.insert(plugin, t),
                            None => self.statuses.remove(&plugin),
                        };
                        continue;
                    }
                    PluginOutput::Panel { plugin, id, lines } => {
                        let lines = lines
                            .into_iter()
                            .map(|l| PanelLine {
                                text: l.text,
                                command: l.command,
                            })
                            .collect();
                        self.panels.insert((plugin, id), lines);
                        continue;
                    }
                    PluginOutput::Notify { plugin, message } => Effect::Notify(format!("{plugin}: {message}")),
                    PluginOutput::Log { plugin, level, message } => {
                        let level = match level {
                            folio_plugin::LogLevel::Warn => crate::log::Level::Warn,
                            folio_plugin::LogLevel::Error => crate::log::Level::Error,
                            _ => crate::log::Level::Info,
                        };
                        crate::log::push(&plugin, level, message);
                        continue;
                    }
                    PluginOutput::Error(e @ (PluginError::Manifest { .. } | PluginError::Load { .. })) => {
                        Effect::Problem(e.to_string())
                    }
                    PluginOutput::Error(e) => Effect::Notify(e.to_string()),
                });
            }
            effects
        }

        pub fn saved(&self, path: &Path) {
            if let Some(r) = &self.runtime {
                r.dispatch(Event::Saved(path.to_path_buf()));
            }
        }

        pub fn run_command(&self, plugin: &str, id: &str) -> Result<(), String> {
            match &self.runtime {
                Some(r) => {
                    r.run_command(plugin, id);
                    Ok(())
                }
                None => Err("Plugins aren't running".into()),
            }
        }

        pub fn commands(&self) -> Vec<PluginCommand> {
            let Some(r) = &self.runtime else { return Vec::new() };
            r.plugins()
                .iter()
                .flat_map(|p| {
                    p.commands.iter().map(|c| PluginCommand {
                        plugin: p.name.clone(),
                        id: c.id.clone(),
                        label: c.label.clone(),
                    })
                })
                .collect()
        }
    }

    fn position(p: Pos) -> Position {
        Position {
            paragraph: p.para as u32,
            character: p.ch as u32,
        }
    }

    fn snapshot(editor: &Editor, path: Option<&Path>, theme: &str) -> DocumentSnapshot {
        let doc = &editor.doc;
        DocumentSnapshot {
            revision: editor.revision,
            title: path
                .and_then(|p| p.file_stem())
                .map_or("Untitled".to_owned(), |s| s.to_string_lossy().into_owned()),
            path: path.map(Path::to_path_buf),
            theme: theme.to_owned(),
            paragraphs: doc.paragraphs.len() as u32,
            words: doc.word_count() as u32,
            characters: doc.paragraphs.iter().map(|p| p.len()).sum::<usize>() as u32,
            selection: editor.selection_range().map(|(a, b)| Selection {
                start: position(a),
                end: position(b),
                text: doc.text(a, b),
            }),
        }
    }
}

#[cfg(not(feature = "wasi-plugins"))]
mod disabled {
    use std::path::Path;

    use super::{Described, Effect, PanelLine, PluginCommand};
    use crate::editor::Editor;

    const NO_PLUGINS: &str = "text9000 Lite doesn't run plugins. Download text9000 (not Lite) to use them";

    /// folio-lite: no plugin runtime.
    pub struct Plugins;

    impl Plugins {
        pub fn start(_ctx: &egui::Context) -> Self {
            Self
        }

        pub fn reload(&mut self) {}

        pub fn describe(_dir: &Path) -> Result<Described, String> {
            Err(NO_PLUGINS.into())
        }

        pub fn install(&mut self, _dir: &Path) -> Result<(), String> {
            Err(NO_PLUGINS.into())
        }

        pub fn status_items(&self) -> Vec<String> {
            Vec::new()
        }

        pub fn panel(&self, _plugin: &str, _id: &str) -> Option<&[PanelLine]> {
            None
        }

        pub fn panel_title(&self, _plugin: &str, _id: &str) -> Option<String> {
            None
        }

        pub fn is_loaded(&self, _plugin: &str) -> bool {
            false
        }

        pub fn loaded(&self) -> Vec<String> {
            Vec::new()
        }

        pub fn update(&mut self, _editor: &Editor, _path: Option<&Path>, _theme: &str) -> Vec<Effect> {
            Vec::new()
        }

        pub fn saved(&self, _path: &Path) {}

        pub fn run_command(&self, _plugin: &str, _id: &str) -> Result<(), String> {
            Err(NO_PLUGINS.into())
        }

        pub fn commands(&self) -> Vec<PluginCommand> {
            Vec::new()
        }
    }
}

#[cfg(all(test, feature = "wasi-plugins"))]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::doc::Document;
    use crate::editor::Editor;

    /// A real plugin's edits and messages reach the editor through
    /// `update`. Uses the probe plugin built by folio_plugin's tests:
    /// `FOLIO_PROBE_WASM=crates/folio_plugin/tests/guest/target/wasm32-wasip2/release/probe_plugin.wasm
    ///  cargo test plugin_reaches_the_editor -- --ignored`
    #[test]
    #[ignore]
    fn plugin_reaches_the_editor() {
        let wasm = std::env::var("FOLIO_PROBE_WASM").unwrap();
        let dir = std::env::temp_dir().join(format!("folio-app-plugins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("probe")).unwrap();
        std::fs::copy(wasm, dir.join("probe/probe.wasm")).unwrap();
        std::fs::write(
            dir.join("probe/plugin.toml"),
            "name = \"Probe\"\nversion = \"1\"\nwasm = \"probe.wasm\"\n\
             [[commands]]\nid = \"hello\"\nlabel = \"Say Hello\"\n\
             [[panels]]\nid = \"goal\"\ntitle = \"Word Goal\"\n\
             [capabilities]\ndocument = [\"read\", \"edit\"]",
        )
        .unwrap();

        let mut plugins = Plugins::start_in(&egui::Context::default(), dir.clone());
        let editor = Editor::new(Document::default());
        let mut effects = Vec::new();
        fn until(plugins: &mut Plugins, editor: &Editor, effects: &mut Vec<Effect>, done: &dyn Fn(&[Effect]) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(effects) {
                assert!(Instant::now() < deadline, "timed out");
                effects.extend(plugins.update(editor, None, "Paper"));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        until(&mut plugins, &editor, &mut effects, &|e| e.iter().any(|e| matches!(e, Effect::CommandsChanged)));
        let commands = plugins.commands();
        assert_eq!((commands[0].plugin.as_str(), commands[0].label.as_str()), ("Probe", "Say Hello"));

        plugins.run_command("Probe", "hello").unwrap();
        until(&mut plugins, &editor, &mut effects, &|e| e.iter().any(|e| matches!(e, Effect::Insert { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::Notify(m) if m == "Probe: Untitled has 0 words")));
        let insert = effects.iter().find_map(|e| match e {
            Effect::Insert { revision, text, .. } => Some((*revision, text.clone())),
            _ => None,
        });
        assert_eq!(insert, Some((editor.revision, "Hi from wasm".to_owned())));

        // Status and panels are kept for the status line and panel view.
        plugins.run_command("Probe", "status:250 to go").unwrap();
        plugins.run_command("Probe", "panel:goal").unwrap();
        until(&mut plugins, &editor, &mut effects, &|e| {
            e.iter().filter(|e| matches!(e, Effect::Notify(m) if m.ends_with("set"))).count() >= 2
        });
        effects.extend(plugins.update(&editor, None, "Paper"));
        assert_eq!(plugins.status_items(), ["250 to go"]);
        assert_eq!(plugins.panel_title("Probe", "goal").as_deref(), Some("Word Goal"));
        assert_eq!(plugins.panel("Probe", "goal").unwrap()[0].text, "Goal: 250 words");

        // Installing another plugin copies it in and reloads.
        let download = dir.with_extension("download");
        let _ = std::fs::remove_dir_all(&download);
        std::fs::create_dir_all(&download).unwrap();
        std::fs::copy(dir.join("probe/probe.wasm"), download.join("probe.wasm")).unwrap();
        std::fs::write(download.join("plugin.toml"), "name = \"Second Probe\"\nversion = \"1\"\nwasm = \"probe.wasm\"").unwrap();
        assert!(Plugins::describe(&download).unwrap().can.is_empty());
        plugins.install(&download).unwrap();
        assert!(dir.join("second-probe/plugin.toml").is_file());
        effects.clear();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !(plugins.is_loaded("Probe") && plugins.is_loaded("Second Probe")) {
            assert!(Instant::now() < deadline, "plugins didn't reload");
            plugins.update(&editor, None, "Paper");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(plugins);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(download);
    }
}
