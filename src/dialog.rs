//! Folio's questions (save before closing, allow this adapter, ...). On
//! macOS and Windows they're the system's own dialogs. On Linux rfd shows
//! them with `zenity`, which KDE and minimal desktops often don't have, so
//! text9000 falls back to KDE's `kdialog`, then to X11's plain `xmessage`.

use rfd::{MessageButtons, MessageDialog, MessageDialogResult};

/// Asks a question and waits for the answer. Closing the dialog counts as
/// Cancel (or No, when there's no Cancel).
pub fn ask(title: &str, text: &str, buttons: MessageButtons) -> MessageDialogResult {
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(answer) = linux::ask(title, text, &buttons) {
        return answer;
    }
    MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_buttons(buttons)
        .show()
}

/// Which program shows questions on Linux, if any can.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn linux_program() -> Option<&'static str> {
    linux::PROGRAMS.into_iter().find(|p| linux::in_path(p))
}

#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use std::process::Command;

    use rfd::{MessageButtons, MessageDialogResult as R};

    pub const PROGRAMS: [&str; 3] = ["zenity", "kdialog", "xmessage"];

    pub fn in_path(program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
    }

    /// The answer from kdialog or xmessage; None means rfd (zenity) should ask.
    pub fn ask(title: &str, text: &str, buttons: &MessageButtons) -> Option<R> {
        let program = super::linux_program()?;
        // (label, what it answers), in the order they're shown.
        let choices: &[(&str, R)] = match buttons {
            MessageButtons::YesNo => &[("Yes", R::Yes), ("No", R::No)],
            MessageButtons::YesNoCancel => &[("Yes", R::Yes), ("No", R::No), ("Cancel", R::Cancel)],
            MessageButtons::OkCancel => &[("OK", R::Ok), ("Cancel", R::Cancel)],
            _ => &[("OK", R::Ok)],
        };
        let closed = choices.last().map_or(R::Ok, |c| c.1.clone());
        let status = match program {
            "kdialog" => {
                let mut cmd = Command::new("kdialog");
                cmd.args(["--title", title]);
                match choices.len() {
                    3 => cmd.args(["--yesnocancel", text]),
                    2 => cmd.args([
                        "--yesno",
                        text,
                        "--yes-label",
                        choices[0].0,
                        "--no-label",
                        choices[1].0,
                    ]),
                    _ => cmd.args(["--msgbox", text]),
                };
                cmd.status()
            }
            "xmessage" => {
                let labels: Vec<String> = choices
                    .iter()
                    .enumerate()
                    .map(|(i, c)| format!("{}:{}", c.0, i + 10))
                    .collect();
                Command::new("xmessage")
                    .args([
                        "-center",
                        "-title",
                        title,
                        "-default",
                        choices[0].0,
                        "-buttons",
                        &labels.join(","),
                    ])
                    .arg(format!("{title}\n\n{text}"))
                    .status()
            }
            _ => return None,
        };
        // kdialog answers 0, 1, 2 for the first, second and third button.
        // xmessage answers the number after each label, from 10 so that its
        // own exit codes (window closed, can't start) never read as "No".
        let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
        let index = if program == "xmessage" {
            code - 10
        } else {
            code
        };
        let answer = usize::try_from(index).ok().and_then(|i| choices.get(i));
        Some(answer.map_or(closed, |c| c.1.clone()))
    }
}
