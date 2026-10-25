//! Finding, compiling and starting plugins.
//!
//! The plugins folder is `plugins/` in Folio's config folder (see
//! [`default_plugins_dir`]). It holds one folder per plugin with a
//! `plugin.toml` and its `.wasm` component:
//!
//! ```text
//! plugins/
//! ├── word-goals/
//! │   ├── plugin.toml
//! │   └── word_goals.wasm
//! └── quick.wasm          a loose component: loads with no capabilities
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, UpdateDeadline};

use crate::PluginError;
use crate::document::PluginOutput;
use crate::host::{Plugin, PluginPre, PluginState};
use crate::manifest::{Capabilities, Command, MANIFEST_FILE, Manifest};
use crate::sandbox::{self, Limits};

/// `plugins/` in Folio's config folder, the same folder as its settings,
/// themes and adapters:
///
/// | | |
/// |---|---|
/// | macOS | `~/Library/Application Support/Folio/plugins` |
/// | Windows | `%APPDATA%\Folio\plugins` |
/// | Linux | `$XDG_CONFIG_HOME/folio/plugins`, usually `~/.config/folio/plugins` |
///
/// `FOLIO_CONFIG_DIR` overrides the config folder, as it does for Folio.
pub fn default_plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FOLIO_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Folio")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map_or_else(|| home.join("AppData/Roaming"), PathBuf::from).join("Folio")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("folio")
    }
}

/// A loaded plugin, as the editor sees it: enough to show it in menus and
/// to ask the person about its capabilities.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub commands: Vec<Command>,
    pub panels: Vec<crate::manifest::Panel>,
    pub capabilities: Capabilities,
    pub dir: PathBuf,
}

/// A plugin found on disk, not yet compiled.
#[derive(Debug, Clone)]
pub struct PluginPackage {
    pub manifest: Manifest,
    /// The plugin's folder (for a loose `.wasm`, the plugins folder).
    pub dir: PathBuf,
    pub wasm: PathBuf,
    /// Where its manifest is, for error messages.
    pub manifest_path: PathBuf,
}

impl PluginPackage {
    pub fn info(&self) -> PluginInfo {
        let m = &self.manifest;
        PluginInfo {
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            author: m.author.clone(),
            commands: m.commands.clone(),
            panels: m.panels.clone(),
            capabilities: m.capabilities.clone(),
            dir: self.dir.clone(),
        }
    }
}

/// Finds every plugin in `dir`. Problems with one plugin don't stop the
/// others; they're returned alongside. A missing folder means no plugins.
pub fn discover(dir: &Path) -> (Vec<PluginPackage>, Vec<PluginError>) {
    let (mut found, mut errors) = (Vec::<PluginPackage>::new(), Vec::new());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (found, errors);
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let package = if path.join(MANIFEST_FILE).is_file() {
            Manifest::load(&path).and_then(|manifest| {
                let wasm = path.join(&manifest.wasm);
                if !wasm.is_file() {
                    return Err(PluginError::Manifest {
                        path: path.join(MANIFEST_FILE),
                        message: format!("{} doesn't exist", manifest.wasm.display()),
                    });
                }
                Ok(PluginPackage {
                    manifest,
                    manifest_path: path.join(MANIFEST_FILE),
                    dir: path.clone(),
                    wasm,
                })
            })
        } else if path.extension().is_some_and(|e| e == "wasm") && path.is_file() {
            let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let text = format!("name = {}\nversion = \"0.0.0\"", toml_string(&name));
            Manifest::parse(&text, &path).map(|manifest| PluginPackage {
                manifest,
                manifest_path: path.clone(),
                dir: dir.to_path_buf(),
                wasm: path.clone(),
            })
        } else {
            continue;
        };
        match package {
            Ok(p) if found.iter().any(|f| f.manifest.name == p.manifest.name) => errors.push(PluginError::Manifest {
                path: p.manifest_path,
                message: format!("another plugin is already named \"{}\"", p.manifest.name),
            }),
            Ok(p) => found.push(p),
            Err(e) => errors.push(e),
        }
    }
    (found, errors)
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_owned()).to_string()
}

/// The shared Wasmtime engine and the linker with Folio's and WASI's
/// imports. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Loader {
    pub engine: Engine,
    linker: Arc<Linker<PluginState>>,
}

impl Loader {
    pub fn new() -> Result<Self, PluginError> {
        let fail = |e: wasmtime::Error| PluginError::Runtime(format!("{e:#}"));
        let mut config = Config::new();
        // Lets a long-running plugin be paused (and stopped at its timeout)
        // without cooperation from the plugin.
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(fail)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(fail)?;
        Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state).map_err(fail)?;
        Ok(Self {
            engine,
            linker: Arc::new(linker),
        })
    }

    /// Compiles a plugin and checks that its imports are ones Folio provides.
    pub fn compile(&self, package: &PluginPackage) -> Result<PluginPre<PluginState>, PluginError> {
        let fail = |e: wasmtime::Error| PluginError::Load {
            plugin: package.manifest.name.clone(),
            message: format!("{e:#}"),
        };
        let component = Component::from_file(&self.engine, &package.wasm).map_err(fail)?;
        let pre = self.linker.instantiate_pre(&component).map_err(fail)?;
        PluginPre::new(pre).map_err(fail)
    }
}

/// One running plugin: its store (memory, WASI state) and its exports.
pub(crate) struct Instance {
    pub store: Store<PluginState>,
    pub bindings: Plugin,
}

/// Starts a fresh instance with its sandbox and limits.
pub(crate) async fn instantiate(
    loader: &Loader,
    pre: &PluginPre<PluginState>,
    package: &PluginPackage,
    outputs: Sender<PluginOutput>,
    limits: &Limits,
) -> Result<Instance, PluginError> {
    let name = package.manifest.name.clone();
    let wasi = sandbox::wasi_context(&package.manifest, &package.manifest_path).await?;
    let state = PluginState::new(
        name.clone(),
        package.manifest.capabilities.clone(),
        &package.manifest.panels,
        wasi,
        outputs,
        limits.clone(),
    );
    let mut store = Store::new(&loader.engine, state);
    store.limiter(|s| &mut s.store_limits);
    // Checked every epoch tick (about 10 ms): yield so other plugins and
    // the runtime keep going, or stop a plugin that's over its time.
    store.epoch_deadline_callback(|ctx| {
        Ok(if ctx.data().timed_out() {
            UpdateDeadline::Interrupt
        } else {
            UpdateDeadline::Yield(1)
        })
    });
    store.set_epoch_deadline(1);
    store.data_mut().begin_event(None);
    let bindings = pre.instantiate_async(&mut store).await.map_err(|e| {
        if store.data().timed_out() {
            PluginError::Timeout {
                plugin: name.clone(),
                limit: store.data().timeout(),
            }
        } else {
            PluginError::Load {
                plugin: name.clone(),
                message: format!("{e:#}"),
            }
        }
    })?;
    Ok(Instance { store, bindings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_live_in_the_config_folder() {
        let dir = default_plugins_dir();
        assert!(dir.ends_with("plugins"));
        let config = dir.parent().unwrap();
        if std::env::var_os("FOLIO_CONFIG_DIR").is_none() {
            let expected = if cfg!(target_os = "macos") {
                "Library/Application Support/Folio"
            } else if cfg!(windows) {
                "Folio"
            } else {
                "folio"
            };
            assert!(config.ends_with(expected), "{}", config.display());
        }
    }

    #[test]
    fn discovers_folders_and_loose_components() {
        let dir = std::env::temp_dir().join(format!("folio-plugin-discover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("good")).unwrap();
        std::fs::write(dir.join("good/plugin.toml"), "name = \"Good\"\nversion = \"1\"\nwasm = \"g.wasm\"").unwrap();
        std::fs::write(dir.join("good/g.wasm"), b"").unwrap();
        std::fs::create_dir_all(dir.join("missing")).unwrap();
        std::fs::write(dir.join("missing/plugin.toml"), "name = \"Missing\"\nversion = \"1\"").unwrap();
        std::fs::create_dir_all(dir.join("twin")).unwrap();
        std::fs::write(dir.join("twin/plugin.toml"), "name = \"Good\"\nversion = \"2\"\nwasm = \"g.wasm\"").unwrap();
        std::fs::write(dir.join("twin/g.wasm"), b"").unwrap();
        std::fs::write(dir.join("loose.wasm"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();

        let (found, errors) = discover(&dir);
        let names: Vec<&str> = found.iter().map(|p| p.manifest.name.as_str()).collect();
        assert_eq!(names, ["Good", "loose"]);
        assert_eq!(found[1].manifest.capabilities, Capabilities::default(), "loose components get nothing");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.to_string().contains("plugin.wasm doesn't exist")));
        assert!(errors.iter().any(|e| e.to_string().contains("already named")));
        assert!(discover(&dir.join("nope")).0.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
