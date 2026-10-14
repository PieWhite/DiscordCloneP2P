//! Windows only: the exe icon, which `tauri-build` embedded before the move to GPUI.
//! Nothing happens for any other target. The application manifest (common controls v6,
//! PerMonitorV2 DPI) comes from GPUI's own `windows-manifest` feature; embedding a second
//! one here makes the link fail with CVT1100 "duplicate resource".

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");

    // The host cfg only says which build-dependencies exist; the target decides.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        if let Err(e) = res.compile() {
            panic!("embedding the Windows icon failed: {e}");
        }
    }
}
