//! Embeds the icon and version info in the Windows .exe (Explorer, taskbar,
//! Alt-Tab). Other targets take the icon from the window or the AppImage.

fn main() {
    println!("cargo:rerun-if-changed=assets/is-alive.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/is-alive.ico");
        res.compile().expect("failed to embed Windows icon");
    }
}
