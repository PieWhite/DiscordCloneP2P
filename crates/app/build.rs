//! Windows only: the exe icon and the application manifest, which `tauri-build` embedded
//! before the move to GPUI. Nothing happens for any other target.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=windows-app.manifest");

    // The host cfg only says which build-dependencies exist; the target decides.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.set_manifest_file("windows-app.manifest");
        if let Err(e) = res.compile() {
            panic!("embedding the Windows icon and manifest failed: {e}");
        }
    }
}
