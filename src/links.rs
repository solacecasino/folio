//! Documents and `folio://` links the system asks Folio to open.
//!
//! - On Windows and Linux they arrive as the program's argument
//!   (`folio "folio://join#…"` or `folio essay.folio`), from the file and
//!   link associations Folio sets up (Windows) or `install.sh` installs
//!   (Linux).
//! - On macOS they arrive as Apple Events: Finder's "open documents" and
//!   the "get URL" event for links. Info.plist declares the `folio` scheme
//!   and the `.folio` document type.
//!
//! A link names what it's for: `folio://join#<invite>` goes to the adapter
//! whose `adapter.toml` lists `links = ["join"]` (Folio Link).

use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    File(PathBuf),
    /// A `folio://` link: its kind ("join") and everything after it.
    Link { kind: String, rest: String },
}

/// Reads one command-line argument or URL.
pub fn parse(arg: &str) -> Incoming {
    match arg.strip_prefix("folio://") {
        Some(link) => {
            let end = link.find(['/', '#', '?']).unwrap_or(link.len());
            let (kind, rest) = link.split_at(end);
            Incoming::Link {
                kind: kind.to_lowercase(),
                rest: rest.trim_start_matches(['/', '#', '?']).to_owned(),
            }
        }
        None => Incoming::File(PathBuf::from(arg)),
    }
}

impl Incoming {
    /// The whole link again, as adapters expect it.
    pub fn url(&self) -> Option<String> {
        match self {
            Incoming::Link { kind, rest } => Some(format!("folio://{kind}#{rest}")),
            Incoming::File(_) => None,
        }
    }
}

static PENDING: Mutex<Vec<Incoming>> = Mutex::new(Vec::new());
static CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

/// Queues something to open and wakes the app.
pub fn deliver(item: Incoming) {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(item);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// What's arrived since the last call.
pub fn take() -> Vec<Incoming> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Listens for macOS's open-document and link events from the start, so the
/// ones that launch Folio aren't missed. Call before the app starts.
pub fn listen_early() {
    #[cfg(target_os = "macos")]
    {
        mac::listen();
        mac::add_open_urls();
    }
}

/// Starts listening (macOS, again, in case starting the app replaced the
/// early handlers) and keeps the system's file and link associations
/// pointing at this copy of Folio (Windows).
pub fn install(ctx: &eframe::egui::Context) {
    let _ = CTX.set(ctx.clone());
    #[cfg(target_os = "macos")]
    mac::listen();
    #[cfg(windows)]
    std::thread::spawn(windows::register);
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AnyThread, class, define_class, msg_send, sel};
    use objc2_foundation::NSString;

    use super::{Incoming, deliver, parse};

    /// Apple Event codes (four-character codes).
    const GURL: u32 = u32::from_be_bytes(*b"GURL");
    const CORE: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and this class
        // doesn't implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "FolioAppleEventHandler"]
        struct Handler;

        impl Handler {
            #[unsafe(method(handleURL:withReply:))]
            fn handle_url(&self, event: &AnyObject, _reply: &AnyObject) {
                // SAFETY: Apple Event descriptors respond to these selectors.
                let url: Option<Retained<NSString>> = unsafe {
                    let direct: Option<Retained<AnyObject>> = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                    direct.and_then(|d| msg_send![&d, stringValue])
                };
                if let Some(url) = url {
                    deliver(parse(&url.to_string()));
                }
            }

            #[unsafe(method(handleOpen:withReply:))]
            fn handle_open(&self, event: &AnyObject, _reply: &AnyObject) {
                // SAFETY: as above; the direct object is a list of file URLs.
                unsafe {
                    let list: Option<Retained<AnyObject>> = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                    let Some(list) = list else { return };
                    let count: isize = msg_send![&list, numberOfItems];
                    for i in 1..=count {
                        let item: Option<Retained<AnyObject>> = msg_send![&list, descriptorAtIndex: i];
                        let url: Option<Retained<AnyObject>> = item.and_then(|d| msg_send![&d, fileURLValue]);
                        let path: Option<Retained<NSString>> = url.and_then(|u| msg_send![&u, path]);
                        if let Some(path) = path {
                            deliver(Incoming::File(path.to_string().into()));
                        }
                    }
                }
            }
        }
    );

    /// `application:openURLs:`, which Cocoa calls on the app delegate for
    /// documents opened from Finder, including the one that launches Folio.
    /// The delegate belongs to the windowing library and has no such method,
    /// so it's added to NSObject (which every delegate inherits from) before
    /// the app starts. Only app delegates are asked for this selector.
    unsafe extern "C-unwind" fn open_urls(_this: &AnyObject, _cmd: objc2::runtime::Sel, _app: &AnyObject, urls: &AnyObject) {
        // SAFETY: `urls` is an NSArray of NSURL.
        unsafe {
            let count: usize = msg_send![urls, count];
            for i in 0..count {
                let url: Retained<AnyObject> = msg_send![urls, objectAtIndex: i];
                let path: Option<Retained<NSString>> = msg_send![&url, path];
                if let Some(path) = path {
                    deliver(Incoming::File(path.to_string().into()));
                }
            }
        }
    }

    pub fn add_open_urls() {
        let sel = sel!(application:openURLs:);
        let class = objc2::runtime::AnyClass::get(c"NSObject").expect("NSObject exists");
        // SAFETY: the function matches the method's signature (v@:@@), and
        // NSObject doesn't have this method already (checked).
        unsafe {
            if objc2::ffi::class_respondsToSelector(class, sel).as_bool() {
                return;
            }
            let imp: objc2::runtime::Imp = std::mem::transmute(
                open_urls as unsafe extern "C-unwind" fn(&AnyObject, objc2::runtime::Sel, &AnyObject, &AnyObject),
            );
            objc2::ffi::class_addMethod(class as *const _ as *mut _, sel, imp, c"v@:@@".as_ptr());
        }
    }

    pub fn listen() {
        // SAFETY: plain init of an NSObject subclass; the handler is kept
        // for the life of the app (the event manager doesn't retain it).
        unsafe {
            let handler: Retained<Handler> = msg_send![Handler::alloc(), init];
            let manager: Retained<AnyObject> = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            let _: () = msg_send![&manager, setEventHandler: &*handler, andSelector: sel!(handleURL:withReply:), forEventClass: GURL, andEventID: GURL];
            let _: () = msg_send![&manager, setEventHandler: &*handler, andSelector: sel!(handleOpen:withReply:), forEventClass: CORE, andEventID: OPEN_DOCUMENTS];
            std::mem::forget(handler);
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::process::Command;

    /// Points `.folio` files and `folio://` links at this folio.exe, for this
    /// user only (no administrator needed). Run each launch, so moving
    /// Folio's folder keeps working.
    pub fn register() {
        let Ok(exe) = std::env::current_exe() else { return };
        let open = format!("\"{}\" \"%1\"", exe.display());
        let icon = format!("{},0", exe.display());
        let keys: [(&str, Option<&str>, &str); 6] = [
            (r"HKCU\Software\Classes\folio", None, "URL:Folio link"),
            (r"HKCU\Software\Classes\folio", Some("URL Protocol"), ""),
            (r"HKCU\Software\Classes\folio\shell\open\command", None, &open),
            (r"HKCU\Software\Classes\.folio", None, "Folio.Document"),
            (r"HKCU\Software\Classes\Folio.Document\shell\open\command", None, &open),
            (r"HKCU\Software\Classes\Folio.Document\DefaultIcon", None, &icon),
        ];
        for (key, name, value) in keys {
            let mut reg = Command::new("reg");
            reg.args(["add", key, "/f", "/d", value]);
            match name {
                Some(n) => reg.args(["/v", n]),
                None => reg.arg("/ve"),
            };
            crate::platform::hide_console(&mut reg);
            let _ = reg.output();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_links_and_files() {
        assert_eq!(
            parse("folio://join#folio-abc"),
            Incoming::Link {
                kind: "join".into(),
                rest: "folio-abc".into()
            }
        );
        assert_eq!(
            parse("folio://JOIN/abc").url().as_deref(),
            Some("folio://join#abc"),
            "a slash works too, and the link is rebuilt for adapters"
        );
        assert_eq!(parse("/docs/essay.folio"), Incoming::File("/docs/essay.folio".into()));
        assert_eq!(parse("folio://join"), Incoming::Link { kind: "join".into(), rest: String::new() });
    }
}
