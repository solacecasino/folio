//! The few things that differ between macOS, Windows and Linux: where the
//! config folder lives, local time, whether a process is alive, opening a
//! folder or link, and the trash. Everything else in Folio is the same code
//! on every platform.

use std::path::{Path, PathBuf};

/// What the system file browser is called, for messages and buttons.
pub const FILE_BROWSER: &str = if cfg!(target_os = "macos") {
    "Finder"
} else if cfg!(windows) {
    "File Explorer"
} else {
    "the file manager"
};

/// What the trash is called.
pub const TRASH: &str = if cfg!(windows) { "the Recycle Bin" } else { "the Trash" };

/// The folder Folio keeps everything in, unless `FOLIO_CONFIG_DIR` says otherwise.
pub fn default_config_dir() -> PathBuf {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Folio")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map_or_else(|| home.join("AppData/Roaming"), PathBuf::from).join("Folio")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("folio")
    }
}

pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

#[cfg(unix)]
pub fn local_time() -> LocalTime {
    // SAFETY: `time` and `localtime_r` only write to the values we pass in.
    let tm = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm
    };
    LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    }
}

#[cfg(windows)]
pub fn local_time() -> LocalTime {
    // SAFETY: GetLocalTime has no preconditions.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    LocalTime {
        year: i32::from(t.wYear),
        month: u32::from(t.wMonth),
        day: u32::from(t.wDay),
        hour: u32::from(t.wHour),
        minute: u32::from(t.wMinute),
        second: u32::from(t.wSecond),
    }
}

#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: the handle is checked, only read from, and closed.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        alive
    }
}

/// Opens a folder in the file browser or a link in the web browser.
pub fn open(target: &std::ffi::OsStr) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(target).spawn().map(|_| ())
}

/// Starts adapters without a console window flashing up on Windows.
pub fn hide_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Moves a folder to the system trash, where it can still be recovered.
#[cfg(target_os = "macos")]
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    let trash = std::env::home_dir().map_or_else(std::env::temp_dir, |h| h.join(".Trash"));
    move_into(dir, &trash)
}

#[cfg(not(target_os = "macos"))]
pub fn move_to_trash(dir: &Path) -> Result<(), String> {
    trash::delete(dir).map_err(|e| format!("Couldn't move {} to {TRASH}: {e}", dir.display()))
}

/// Moves `dir` into the folder `into`, renaming it if the name is taken.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn move_into(dir: &Path, into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let name = dir.file_name().map_or("adapter".into(), |n| n.to_string_lossy().into_owned());
    let mut dest = into.join(&name);
    if dest.exists() {
        dest = into.join(format!("{name} {}", crate::safety::local_timestamp().replace(':', ".")));
    }
    // Renaming fails across volumes; copy and delete instead.
    std::fs::rename(dir, &dest)
        .or_else(|_| copy_dir(dir, &dest).and_then(|()| std::fs::remove_dir_all(dir)))
        .map_err(|e| format!("Couldn't move {} to {TRASH}: {e}", dir.display()))
}

/// Copies a folder and everything in it. File permissions (like a
/// program's executable bit) are kept.
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Why Folio's questions (save before closing, allow this adapter, ...)
/// can't be shown, if they can't. On Linux they're shown by `zenity`; with
/// it missing, every question would quietly come back as "Cancel".
pub fn dialog_problem() -> Option<String> {
    if cfg!(any(target_os = "macos", windows)) {
        return None;
    }
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("zenity").is_file()));
    (!found).then(|| {
        "Folio can't show its questions (like saving before closing) because zenity isn't installed. \
         Install it: sudo apt install zenity (Debian, Ubuntu), sudo dnf install zenity (Fedora), \
         sudo pacman -S zenity (Arch)."
            .to_owned()
    })
}

/// On Linux under X11, the windowing library needs libxkbcommon-x11 and
/// crashes with a cryptic message without it. Checked before the window
/// opens so Folio can say what to install instead.
pub fn missing_system_library() -> Option<String> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        if !wayland && std::env::var_os("DISPLAY").is_some() {
            // SAFETY: dlopen with a valid C string; the handle is closed again.
            let found = unsafe {
                let handle = libc::dlopen(c"libxkbcommon-x11.so.0".as_ptr(), libc::RTLD_LAZY);
                if !handle.is_null() {
                    libc::dlclose(handle);
                }
                !handle.is_null()
            };
            if !found {
                return Some(
                    "Folio needs the libxkbcommon-x11 library to open its window. Install it: \
                     sudo apt install libxkbcommon-x11-0 (Debian, Ubuntu), \
                     sudo dnf install libxkbcommon-x11 (Fedora), sudo pacman -S libxkbcommon-x11 (Arch)."
                        .to_owned(),
                );
            }
        }
    }
    None
}

/// Where the system keeps fonts, for themes that name a font file without a
/// folder (`"Georgia.ttf"`), so one theme works on every platform.
pub fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    if cfg!(target_os = "macos") {
        vec![
            "/System/Library/Fonts".into(),
            "/Library/Fonts".into(),
            home.join("Library/Fonts"),
        ]
    } else if cfg!(windows) {
        let windir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        let mut dirs = vec![windir.join("Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
        dirs
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map_or_else(|| home.join(".local/share"), PathBuf::from);
        vec![
            "/usr/share/fonts".into(),
            "/usr/local/share/fonts".into(),
            data.join("fonts"),
            home.join(".fonts"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive() {
        assert!(process_alive(std::process::id()));
        let t = local_time();
        assert!(t.year >= 2024 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
    }

    #[test]
    fn copies_and_moves_folders() {
        let root = std::env::temp_dir().join(format!("folio-platform-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/inner")).unwrap();
        std::fs::write(root.join("a/inner/f.txt"), "hi").unwrap();
        copy_dir(&root.join("a"), &root.join("b")).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("b/inner/f.txt")).unwrap(), "hi");
        move_into(&root.join("b"), &root.join("bin")).unwrap();
        assert!(!root.join("b").exists() && root.join("bin/b/inner/f.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
