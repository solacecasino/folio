//! `plugin.toml`: what a plugin is, which events it wants, and which
//! capabilities it asks for.
//!
//! ```toml
//! name = "Word Goals"
//! version = "0.1.0"
//! description = "A word-count goal for each document."
//! wasm = "word_goals.wasm"          # default: plugin.wasm
//! events = ["changed"]              # also "saved", "selection-changed"
//!
//! [[commands]]
//! id = "set-goal"
//! label = "Set Word Goal"
//!
//! [capabilities]
//! document = ["read", "edit"]       # selection and metadata; inserting text
//! filesystem = [{ path = "~/Notes", access = "read" }]   # seen inside as /Notes
//! network = ["api.example.com:443"] # or "any"
//! ```
//!
//! Everything in `[capabilities]` is denied unless listed. Unknown keys are
//! errors rather than ignored, so a typo can't silently grant or drop a
//! capability.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::PluginError;
use crate::document::Event;

pub const MANIFEST_FILE: &str = "plugin.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// The component file, relative to the plugin's folder.
    #[serde(default = "default_wasm")]
    pub wasm: PathBuf,
    /// Events to receive besides `startup` and the plugin's own commands.
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

fn default_wasm() -> PathBuf {
    "plugin.wasm".into()
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub label: String,
}

/// What a plugin may do. The default denies everything.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub document: Vec<DocumentAccess>,
    #[serde(default)]
    pub filesystem: Vec<FilesystemGrant>,
    #[serde(default)]
    pub network: Network,
}

impl Capabilities {
    pub fn can_read_document(&self) -> bool {
        self.document.contains(&DocumentAccess::Read)
    }

    pub fn can_edit_document(&self) -> bool {
        self.document.contains(&DocumentAccess::Edit)
    }

    /// One line per capability, for asking the person before a plugin runs.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.can_read_document() {
            out.push("read the open document".to_owned());
        }
        if self.can_edit_document() {
            out.push("edit the open document".to_owned());
        }
        for g in &self.filesystem {
            let verb = match g.access {
                Access::Read => "read",
                Access::ReadWrite => "read and change",
            };
            out.push(format!("{verb} files in {}", g.path));
        }
        match &self.network {
            Network::None => {}
            Network::Any => out.push("connect to any address on the internet".to_owned()),
            Network::Hosts(h) => out.push(format!("connect to {}", h.join(", "))),
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocumentAccess {
    Read,
    Edit,
}

/// A folder the plugin may use. It sees it at `mount`, by default `/` plus
/// the folder's name.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilesystemGrant {
    /// A folder on this computer: absolute, or starting with `~/`.
    pub path: String,
    #[serde(default)]
    pub access: Access,
    /// Where the plugin sees it, such as `/notes`.
    #[serde(default)]
    pub mount: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    Read,
    ReadWrite,
}

/// Where the plugin may connect.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Network {
    /// Nothing (the default).
    #[default]
    None,
    /// Any address.
    Any,
    /// Only these `host:port` pairs, resolved when the plugin starts.
    Hosts(Vec<String>),
}

impl<'de> Deserialize<'de> for Network {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Word(String),
            List(Vec<String>),
        }
        match Raw::deserialize(d)? {
            Raw::Word(w) if w == "any" => Ok(Network::Any),
            Raw::Word(w) if w == "none" => Ok(Network::None),
            Raw::Word(w) => Err(serde::de::Error::custom(format!(
                "network must be \"any\", \"none\" or a list of \"host:port\", not \"{w}\""
            ))),
            Raw::List(l) if l.is_empty() => Ok(Network::None),
            Raw::List(l) => Ok(Network::Hosts(l)),
        }
    }
}

impl FilesystemGrant {
    /// The folder on this computer.
    pub fn host_path(&self) -> PathBuf {
        match self.path.strip_prefix("~/") {
            Some(rest) => std::env::home_dir().unwrap_or_default().join(rest),
            None => PathBuf::from(&self.path),
        }
    }

    /// Where the plugin sees it.
    pub fn guest_path(&self) -> String {
        self.mount.clone().unwrap_or_else(|| {
            let name = self.host_path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("/{name}")
        })
    }
}

impl Manifest {
    /// Reads and checks `plugin.toml` in a plugin's folder.
    pub fn load(dir: &Path) -> Result<Self, PluginError> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).map_err(|e| PluginError::Manifest {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Self::parse(&text, &path)
    }

    pub fn parse(text: &str, path: &Path) -> Result<Self, PluginError> {
        let fail = |message: String| PluginError::Manifest {
            path: path.to_path_buf(),
            message,
        };
        let manifest: Manifest = toml::from_str(text).map_err(|e| fail(e.message().to_owned()))?;
        manifest.validate().map_err(fail)?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("`name` can't be empty".into());
        }
        if self.wasm.is_absolute() || self.wasm.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err("`wasm` must be a file inside the plugin's folder".into());
        }
        for e in &self.events {
            if !Event::SUBSCRIBABLE.contains(&e.as_str()) {
                return Err(format!(
                    "unknown event \"{e}\" (events are {})",
                    Event::SUBSCRIBABLE.join(", ")
                ));
            }
        }
        let mut ids = std::collections::HashSet::new();
        for c in &self.commands {
            if c.id.is_empty() || !ids.insert(&c.id) {
                return Err(format!("command ids must be unique and not empty (\"{}\")", c.id));
            }
        }
        for g in &self.capabilities.filesystem {
            if !(g.path.starts_with("~/") || Path::new(&g.path).is_absolute()) {
                return Err(format!("filesystem path \"{}\" must be absolute or start with ~/", g.path));
            }
            let mount = g.guest_path();
            if !mount.starts_with('/') || mount.contains("..") {
                return Err(format!("mount \"{mount}\" must be an absolute path like /notes"));
            }
        }
        if let Network::Hosts(hosts) = &self.capabilities.network {
            for h in hosts {
                let ok = h.rsplit_once(':').is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok());
                if !ok {
                    return Err(format!("network entry \"{h}\" must look like host:port"));
                }
            }
        }
        Ok(())
    }

    pub fn wants(&self, event: &Event) -> bool {
        event.always_delivered() || self.events.iter().any(|e| e == event.kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Manifest, PluginError> {
        Manifest::parse(text, Path::new("plugin.toml"))
    }

    #[test]
    fn everything_is_denied_by_default() {
        let m = parse("name = \"A\"\nversion = \"1\"").unwrap();
        assert_eq!(m.capabilities, Capabilities::default());
        assert!(!m.capabilities.can_read_document() && !m.capabilities.can_edit_document());
        assert_eq!(m.capabilities.network, Network::None);
        assert!(m.capabilities.describe().is_empty());
        assert_eq!(m.wasm, PathBuf::from("plugin.wasm"));
        assert!(m.wants(&Event::Startup) && m.wants(&Event::Command("x".into())));
        assert!(!m.wants(&Event::Changed), "only subscribed events arrive");
    }

    #[test]
    fn reads_capabilities() {
        let m = parse(
            r#"
            name = "A"
            version = "1"
            events = ["changed"]
            [capabilities]
            document = ["read", "edit"]
            filesystem = [{ path = "/tmp/notes", access = "read-write", mount = "/n" }, { path = "~/Docs" }]
            network = ["example.com:443"]
            "#,
        )
        .unwrap();
        let c = &m.capabilities;
        assert!(c.can_read_document() && c.can_edit_document());
        assert_eq!(c.filesystem[0].access, Access::ReadWrite);
        assert_eq!(c.filesystem[0].guest_path(), "/n");
        assert_eq!(c.filesystem[1].access, Access::Read);
        assert_eq!(c.filesystem[1].guest_path(), "/Docs");
        assert_eq!(c.network, Network::Hosts(vec!["example.com:443".into()]));
        assert_eq!(c.describe().len(), 5);
        assert!(m.wants(&Event::Changed));
        assert_eq!(parse("name=\"A\"\nversion=\"1\"\n[capabilities]\nnetwork=\"any\"").unwrap().capabilities.network, Network::Any);
    }

    #[test]
    fn typos_and_bad_values_are_errors() {
        let bad = [
            ("name = \"A\"\nversion = \"1\"\n[capabilities]\nnetwrok = \"any\"", "netwrok"),
            ("name = \"A\"\nversion = \"1\"\n[capabilities]\nnetwork = \"everything\"", "network must be"),
            ("name = \"A\"\nversion = \"1\"\n[capabilities]\nnetwork = [\"example.com\"]", "host:port"),
            ("name = \"A\"\nversion = \"1\"\n[capabilities]\nfilesystem = [{ path = \"notes\" }]", "absolute"),
            ("name = \"A\"\nversion = \"1\"\n[capabilities]\ndocument = [\"delete\"]", "delete"),
            ("name = \"A\"\nversion = \"1\"\nevents = [\"keypress\"]", "unknown event"),
            ("name = \"A\"\nversion = \"1\"\nwasm = \"../x.wasm\"", "inside the plugin"),
            ("name = \"\"\nversion = \"1\"", "empty"),
        ];
        for (text, needle) in bad {
            let err = parse(text).unwrap_err().to_string();
            assert!(err.contains(needle), "{text:?} gave {err}");
        }
    }
}
