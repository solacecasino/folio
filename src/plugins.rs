//! WebAssembly plugins: sandboxed extensions that run inside Folio (see
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
    /// A short message to show.
    Notify(String),
    /// A plugin that couldn't load (shown until fixed).
    Problem(String),
    /// Plugins and their commands changed; rebuild menus.
    CommandsChanged,
}

/// One plugin command, for menus and key bindings.
pub struct PluginCommand {
    pub plugin: String,
    pub id: String,
    pub label: String,
}

/// The version as About shows it; folio-lite builds say so.
pub fn version() -> String {
    let lite = if cfg!(feature = "wasi-plugins") { "" } else { " (lite)" };
    format!("{}{lite}", env!("CARGO_PKG_VERSION"))
}

#[cfg(feature = "wasi-plugins")]
pub use enabled::Plugins;

#[cfg(not(feature = "wasi-plugins"))]
pub use disabled::Plugins;

#[cfg(feature = "wasi-plugins")]
mod enabled {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use eframe::egui;
    use folio_plugin::{
        DocumentSnapshot, Edit, Event, PluginError, PluginOutput, PluginRuntime, Position, RuntimeConfig, Selection,
    };

    use super::{Effect, PluginCommand};
    use crate::doc::Pos;
    use crate::editor::Editor;

    pub struct Plugins {
        /// None if the runtime couldn't start; the problem was reported.
        runtime: Option<PluginRuntime>,
        startup_problem: Option<String>,
        /// (revision, cursor, anchor) last published, to send only changes.
        sent: Option<(u64, Pos, Pos)>,
        path: Option<PathBuf>,
    }

    impl Plugins {
        pub fn start(ctx: &egui::Context) -> Self {
            Self::start_in(ctx, crate::settings::config_dir().join("plugins"))
        }

        pub fn start_in(ctx: &egui::Context, plugins_dir: PathBuf) -> Self {
            let ctx = ctx.clone();
            let config = RuntimeConfig {
                plugins_dir,
                waker: Some(Arc::new(move || ctx.request_repaint())),
                ..Default::default()
            };
            match PluginRuntime::start(config) {
                Ok(runtime) => Self {
                    runtime: Some(runtime),
                    startup_problem: None,
                    sent: None,
                    path: None,
                },
                Err(e) => Self {
                    runtime: None,
                    startup_problem: Some(format!("Plugins are off: {e}")),
                    sent: None,
                    path: None,
                },
            }
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
                    PluginOutput::Notify { plugin, message } => Effect::Notify(format!("{plugin}: {message}")),
                    PluginOutput::Log { plugin, level, message } => {
                        eprintln!("folio: plugin {plugin} ({level:?}): {message}");
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

    use eframe::egui;

    use super::{Effect, PluginCommand};
    use crate::editor::Editor;

    /// folio-lite: no plugin runtime.
    pub struct Plugins;

    impl Plugins {
        pub fn start(_ctx: &egui::Context) -> Self {
            Self
        }

        pub fn update(&mut self, _editor: &Editor, _path: Option<&Path>, _theme: &str) -> Vec<Effect> {
            Vec::new()
        }

        pub fn saved(&self, _path: &Path) {}

        pub fn run_command(&self, _plugin: &str, _id: &str) -> Result<(), String> {
            Err("This build of Folio (folio-lite) doesn't include plugins".into())
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
             [capabilities]\ndocument = [\"read\", \"edit\"]",
        )
        .unwrap();

        let mut plugins = Plugins::start_in(&eframe::egui::Context::default(), dir.clone());
        let editor = Editor::new(Document::default());
        let mut effects = Vec::new();
        let mut until = |plugins: &mut Plugins, done: &dyn Fn(&[Effect]) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(&effects) {
                assert!(Instant::now() < deadline, "timed out");
                effects.extend(plugins.update(&editor, None, "Paper"));
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        until(&mut plugins, &|e| e.iter().any(|e| matches!(e, Effect::CommandsChanged)));
        let commands = plugins.commands();
        assert_eq!((commands[0].plugin.as_str(), commands[0].label.as_str()), ("Probe", "Say Hello"));

        plugins.run_command("Probe", "hello").unwrap();
        until(&mut plugins, &|e| e.iter().any(|e| matches!(e, Effect::Insert { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::Notify(m) if m == "Probe: Untitled has 0 words")));
        let insert = effects.iter().find_map(|e| match e {
            Effect::Insert { revision, text, .. } => Some((*revision, text.clone())),
            _ => None,
        });
        assert_eq!(insert, Some((editor.revision, "Hi from wasm".to_owned())));
        drop(plugins);
        let _ = std::fs::remove_dir_all(dir);
    }
}
