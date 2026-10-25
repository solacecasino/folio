//! Folio's log: what adapters and plugins print, their errors, and Folio's
//! own messages, shown in **View → Log** so people writing adapters and
//! plugins don't need to start Folio from a terminal. Lines are also
//! written to stderr as before.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Lines kept; older ones are dropped.
const KEEP: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Line {
    /// Local time, HH:MM:SS.
    pub time: String,
    /// "Folio", or the adapter's or plugin's name.
    pub source: String,
    pub level: Level,
    pub text: String,
}

static LINES: Mutex<VecDeque<Line>> = Mutex::new(VecDeque::new());

/// Adds a line. Safe to call from any thread.
pub fn push(source: &str, level: Level, text: impl Into<String>) {
    let text = text.into();
    eprintln!("folio: {source}: {text}");
    // FOLIO_LOG_FILE also appends every line to a file, for debugging a
    // Folio started by the system (from a link or Finder).
    if let Some(path) = std::env::var_os("FOLIO_LOG_FILE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{source}: {text}");
        }
    }
    let t = crate::platform::local_time();
    let line = Line {
        time: format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second),
        source: source.to_owned(),
        level,
        text,
    };
    let mut lines = LINES.lock().unwrap_or_else(|e| e.into_inner());
    if lines.len() >= KEEP {
        lines.pop_front();
    }
    lines.push_back(line);
}

pub fn lines() -> Vec<Line> {
    LINES.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

pub fn clear() {
    LINES.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The log window's state: which source to show.
#[derive(Default)]
pub struct Window {
    pub filter: Option<String>,
}

impl Window {
    /// Draws the window; returns false when it was closed.
    pub fn show(&mut self, ctx: &eframe::egui::Context) -> bool {
        use eframe::egui::{self, Color32, RichText};
        let mut open = true;
        let lines = lines();
        let mut sources: Vec<String> = lines.iter().map(|l| l.source.clone()).collect();
        sources.sort();
        sources.dedup();
        egui::Window::new("Log").open(&mut open).default_size([640.0, 360.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("log-filter")
                    .selected_text(self.filter.as_deref().unwrap_or("Everything"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filter, None, "Everything");
                        for s in &sources {
                            ui.selectable_value(&mut self.filter, Some(s.clone()), s);
                        }
                    });
                if ui.button("Copy").clicked() {
                    let text: Vec<String> = lines
                        .iter()
                        .filter(|l| self.filter.as_ref().is_none_or(|f| f == &l.source))
                        .map(|l| format!("{} {}: {}", l.time, l.source, l.text))
                        .collect();
                    ui.ctx().copy_text(text.join("\n"));
                }
                if ui.button("Clear").clicked() {
                    clear();
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                let shown: Vec<&Line> = lines
                    .iter()
                    .filter(|l| self.filter.as_ref().is_none_or(|f| f == &l.source))
                    .collect();
                if shown.is_empty() {
                    ui.weak("Nothing yet. Adapters' and plugins' output and errors appear here.");
                }
                for l in shown {
                    let color = match l.level {
                        Level::Info => ui.visuals().text_color(),
                        Level::Warn => Color32::from_rgb(220, 160, 60),
                        Level::Error => Color32::from_rgb(220, 90, 80),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&l.time).weak().monospace());
                        ui.label(RichText::new(&l.source).strong());
                        ui.label(RichText::new(&l.text).color(color).monospace());
                    });
                }
            });
        });
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_lines() {
        for i in 0..KEEP + 5 {
            push("Test", Level::Info, format!("line {i}"));
        }
        let lines = lines();
        assert!(lines.len() <= KEEP);
        assert_eq!(lines.last().unwrap().text, format!("line {}", KEEP + 4));
    }
}
