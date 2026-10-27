//! Lua scripts: `init.lua` in the config folder, run once at startup and
//! again on Reload Config (or when the file changes).
//!
//! Scripts get a sandbox: the string, table, math, utf8 and coroutine
//! libraries, a clock (`os.time`, `os.clock`, `os.date`), and the `folio`
//! table. No files, no programs, no network, no loading other modules. Each
//! run is capped at 1 second and the whole state at 32 MB, and every error
//! goes to the Log instead of stopping Folio. Without an init.lua no Lua
//! state exists at all.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use mlua::{Function, HookTriggers, Lua, LuaOptions, RegistryKey, StdLib, Value, VmState};

use crate::editor::Editor;
use crate::log;
use crate::theme::Theme;

/// How long one run (init.lua, or one command) may take.
const TIME_LIMIT: Duration = Duration::from_secs(1);
const MEMORY_LIMIT: usize = 32 << 20;

pub fn init_path() -> PathBuf {
    crate::settings::config_dir().join("init.lua")
}

#[derive(Default)]
pub struct Scripts {
    lua: Option<Lua>,
    /// Commands registered with `folio.register_command`, by name.
    commands: Rc<RefCell<BTreeMap<String, RegistryKey>>>,
    /// When the current run has to stop.
    deadline: Rc<Cell<Option<Instant>>>,
    /// init.lua's modification time when it was last run.
    loaded: Option<SystemTime>,
}

impl Scripts {
    /// Runs init.lua again if it changed (or appeared, or went away) since
    /// it last ran.
    pub fn reload_if_changed(&mut self, editor: &mut Editor, theme: &Theme) -> Result<(), String> {
        let modified = std::fs::metadata(init_path()).and_then(|m| m.modified()).ok();
        if modified == self.loaded {
            return Ok(());
        }
        self.load(editor, theme)
    }

    /// Starts a fresh Lua state and runs init.lua, if there is one. An error
    /// is logged (once, here) and returned for the status line.
    pub fn load(&mut self, editor: &mut Editor, theme: &Theme) -> Result<(), String> {
        let result = self.start(editor, theme);
        if let Err(e) = &result {
            log::push("Lua", log::Level::Error, e.clone());
        }
        result
    }

    fn start(&mut self, editor: &mut Editor, theme: &Theme) -> Result<(), String> {
        self.lua = None;
        self.commands.borrow_mut().clear();
        let path = init_path();
        self.loaded = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
        };
        let lua = self.sandbox().map_err(|e| format!("Couldn't start Lua: {e}"))?;
        let chunk = lua.load(source).set_name("@init.lua");
        let result = self.with_api(&lua, editor, theme, |_| chunk.exec());
        // Commands registered before an error still work.
        self.lua = Some(lua);
        result.map_err(|e| describe(&e))?;
        let names = self.names();
        let detail = if names.is_empty() { String::new() } else { format!(" (commands: {})", names.join(", ")) };
        log::push("Lua", log::Level::Info, format!("Ran init.lua{detail}"));
        Ok(())
    }

    /// Runs a registered command. An error is logged (once, here) and
    /// returned for the status line.
    pub fn run(&mut self, name: &str, editor: &mut Editor, theme: &Theme) -> Result<(), String> {
        let Some(lua) = self.lua.take() else {
            let e = format!("No Lua command \"{name}\": there's no init.lua, or it didn't load");
            log::push("Lua", log::Level::Error, e.clone());
            return Err(e);
        };
        let function = {
            let commands = self.commands.borrow();
            commands.get(name).map(|key| lua.registry_value::<Function>(key))
        };
        let result = match function {
            None => Err(match self.names() {
                names if names.is_empty() => format!("No Lua command \"{name}\": init.lua registers none"),
                names => format!("No Lua command \"{name}\" (init.lua registers: {})", names.join(", ")),
            }),
            Some(Err(e)) => Err(describe(&e)),
            Some(Ok(f)) => self.with_api(&lua, editor, theme, |_| f.call::<()>(())).map_err(|e| describe(&e)),
        };
        self.lua = Some(lua);
        if let Err(e) = &result {
            log::push("Lua", log::Level::Error, e.clone());
        }
        result
    }

    pub fn names(&self) -> Vec<String> {
        self.commands.borrow().keys().cloned().collect()
    }

    /// A Lua state with only the safe libraries, `print` going to the Log,
    /// and the limits set.
    fn sandbox(&self) -> mlua::Result<Lua> {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE | StdLib::OS;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(MEMORY_LIMIT)?;
        let globals = lua.globals();

        // The os library is only for the clock.
        let os: mlua::Table = globals.get("os")?;
        let clock = lua.create_table()?;
        for name in ["time", "clock", "date", "difftime"] {
            clock.set(name, os.get::<Value>(name)?)?;
        }
        globals.set("os", clock)?;
        // The base library can read files and load precompiled code.
        for name in ["dofile", "loadfile", "require", "collectgarbage"] {
            globals.set(name, Value::Nil)?;
        }
        lua.load(
            r#"local load_ = load
            load = function(chunk, name, _, env) return load_(chunk, name, "t", env) end"#,
        )
        .set_name("=sandbox")
        .exec()?;
        globals.set(
            "print",
            lua.create_function(|_, args: mlua::Variadic<Value>| {
                let line: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_else(|_| format!("{v:?}"))).collect();
                log::push("Lua", log::Level::Info, line.join("\t"));
                Ok(())
            })?,
        )?;

        let deadline = self.deadline.clone();
        lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
            match deadline.get() {
                Some(at) if Instant::now() > at => {
                    Err(mlua::Error::runtime(format!("stopped: took longer than {} second", TIME_LIMIT.as_secs())))
                }
                _ => Ok(VmState::Continue),
            }
        })?;
        Ok(lua)
    }

    /// Runs `f` with the `folio` table bound to this editor. The functions
    /// borrow the editor only for the length of the run.
    fn with_api<R>(
        &self,
        lua: &Lua,
        editor: &mut Editor,
        theme: &Theme,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let editor = RefCell::new(editor);
        self.deadline.set(Some(Instant::now() + TIME_LIMIT));
        let result = lua.scope(|scope| {
            let folio = lua.create_table()?;
            folio.set(
                "get_text",
                scope.create_function(|_, ()| Ok(editor.borrow().doc.plain_text()))?,
            )?;
            folio.set(
                "set_text",
                scope.create_function(|_, text: String| {
                    editor.borrow_mut().set_text(&text, theme);
                    Ok(())
                })?,
            )?;
            folio.set(
                "get_cursor",
                scope.create_function(|_, ()| {
                    let ed = editor.borrow();
                    Ok(ed.doc.index_of(ed.cursor()))
                })?,
            )?;
            folio.set(
                "set_cursor",
                scope.create_function(|_, index: i64| {
                    let mut ed = editor.borrow_mut();
                    let pos = ed.doc.pos_at(index.max(0) as usize);
                    ed.move_caret(pos);
                    Ok(())
                })?,
            )?;
            folio.set(
                "get_selection",
                scope.create_function(|_, ()| {
                    let ed = editor.borrow();
                    let (a, b) = (ed.doc.index_of(ed.anchor()), ed.doc.index_of(ed.cursor()));
                    let text = ed.selected_text().unwrap_or_default();
                    Ok((a.min(b), a.max(b), text))
                })?,
            )?;
            folio.set(
                "set_selection",
                scope.create_function(|_, (from, to): (i64, Option<i64>)| {
                    let mut ed = editor.borrow_mut();
                    let from = ed.doc.pos_at(from.max(0) as usize);
                    let to = to.map_or(from, |to| ed.doc.pos_at(to.max(0) as usize));
                    ed.select(from, to);
                    Ok(())
                })?,
            )?;
            folio.set(
                "insert_text",
                scope.create_function(|_, text: String| {
                    editor.borrow_mut().insert_at_cursor(&text, theme);
                    Ok(())
                })?,
            )?;
            let commands = self.commands.clone();
            folio.set(
                "register_command",
                lua.create_function(move |lua, (name, f): (String, Function)| {
                    let name = name.trim().to_owned();
                    if name.is_empty() || name.contains(char::is_whitespace) {
                        return Err(mlua::Error::runtime(format!("command names can't be empty or contain spaces: \"{name}\"")));
                    }
                    commands.borrow_mut().insert(name, lua.create_registry_value(f)?);
                    Ok(())
                })?,
            )?;
            lua.globals().set("folio", folio)?;
            f(lua)
        });
        self.deadline.set(None);
        result
    }
}

/// A Lua error as one readable message, with the script line.
fn describe(e: &mlua::Error) -> String {
    match e {
        mlua::Error::CallbackError { traceback, cause } => format!("{}\n{traceback}", describe(cause)),
        mlua::Error::SyntaxError { message, .. } => format!("Syntax error: {message}"),
        mlua::Error::MemoryError(_) => format!("Out of memory: scripts get {} MB", MEMORY_LIMIT >> 20),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Document, Pos};

    fn theme() -> Theme {
        toml::from_str(crate::theme::DEFAULT_THEME).unwrap()
    }

    fn run(source: &str, editor: &mut Editor) -> mlua::Result<Scripts> {
        let theme = theme();
        let scripts = Scripts::default();
        let lua = scripts.sandbox()?;
        scripts.with_api(&lua, editor, &theme, |lua| lua.load(source).exec())?;
        let mut scripts = scripts;
        scripts.lua = Some(lua);
        Ok(scripts)
    }

    fn editor(text: &str) -> Editor {
        let mut ed = Editor::new(Document::default());
        ed.set_text(text, &theme());
        ed
    }

    #[test]
    fn the_api_reads_and_edits_the_document() {
        let mut ed = editor("one\ntwo");
        run(
            r#"
            assert(folio.get_text() == "one\ntwo")
            folio.set_cursor(4)
            assert(folio.get_cursor() == 4)
            folio.insert_text("2: ")
            "#,
            &mut ed,
        )
        .unwrap();
        assert_eq!(ed.doc.plain_text(), "one\n2: two");
        assert_eq!(ed.cursor(), Pos::new(1, 3));
        assert!(ed.dirty);
        run(r#"folio.set_text("fresh") folio.set_cursor(99)"#, &mut ed).unwrap();
        assert_eq!(ed.doc.plain_text(), "fresh");
        assert_eq!(ed.cursor(), Pos::new(0, 5));
    }

    #[test]
    fn selections_are_read_and_set() {
        let mut ed = editor("héllo\nworld");
        run(
            r#"
            local a, b, text = folio.get_selection()
            assert(a == b and text == "", "nothing selected yet")
            folio.set_selection(9, 3)
            a, b, text = folio.get_selection()
            assert(a == 3 and b == 9, a .. " " .. b)
            assert(text == "lo\nwor", text)
            assert(folio.get_cursor() == 3, "the caret is at the second index")
            folio.set_selection(1, 5)
            folio.insert_text("ELLO")
            assert(folio.get_text() == "hELLO\nworld", folio.get_text())
            folio.set_selection(2)
            a, b = folio.get_selection()
            assert(a == 2 and b == 2)
            "#,
            &mut ed,
        )
        .unwrap();
    }

    #[test]
    fn the_shipped_example_runs() {
        let mut ed = editor("say hi");
        let mut scripts = run(crate::settings::EXAMPLE_LUA, &mut ed).unwrap();
        assert_eq!(scripts.names(), vec!["date", "quote"]);
        ed.select(crate::doc::Pos::new(0, 4), crate::doc::Pos::new(0, 6));
        scripts.run("quote", &mut ed, &theme()).unwrap();
        assert_eq!(ed.doc.plain_text(), "say “hi”");
    }

    #[test]
    fn commands_run_later_against_the_current_document() {
        let mut ed = editor("hello");
        let mut scripts = run(
            r#"folio.register_command("shout", function() folio.set_text(folio.get_text():upper()) end)"#,
            &mut ed,
        )
        .unwrap();
        assert_eq!(scripts.names(), vec!["shout"]);
        scripts.run("shout", &mut ed, &theme()).unwrap();
        assert_eq!(ed.doc.plain_text(), "HELLO");
        assert!(scripts.run("missing", &mut ed, &theme()).is_err());
    }

    #[test]
    fn scripts_cannot_reach_files_programs_or_modules() {
        let mut ed = editor("");
        run(
            r#"
            assert(io == nil and package == nil and debug == nil and require == nil)
            assert(dofile == nil and loadfile == nil)
            assert(os.execute == nil and os.remove == nil and os.rename == nil and os.getenv == nil and os.exit == nil)
            assert(type(os.time()) == "number")
            assert(load(string.dump(function() end)) == nil, "precompiled code is refused")
            assert(load("return 1 + 1")() == 2)
            "#,
            &mut ed,
        )
        .unwrap();
    }

    #[test]
    fn errors_and_runaway_scripts_are_stopped_not_fatal() {
        let mut ed = editor("");
        assert!(run("this is not lua", &mut ed).is_err());
        let started = Instant::now();
        let Err(e) = run("while true do end", &mut ed) else { panic!("an endless loop must be stopped") };
        assert!(describe(&e).contains("took longer"), "{}", describe(&e));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(run("local t = {} for i = 1, 1e9 do t[i] = ('x'):rep(1000) .. i end", &mut ed).is_err());
    }
}
