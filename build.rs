//! Gives text9000.exe its icon on Windows. Nothing to do elsewhere.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Only when building on Windows for Windows: the resource compiler is
    // part of the Windows SDK.
    if cfg!(windows) && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "text9000");
        res.set("FileDescription", "text9000");
        res.compile().expect("couldn't add the icon to text9000.exe");
    }
}
