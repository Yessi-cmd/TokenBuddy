//! Generates the Tauri context (icons, capabilities, config) at build time.
//!
//! On Windows the Common Controls v6 manifest is normally embedded by
//! tauri-build, but only into `bin` targets: tauri-winres emits
//! `rustc-link-arg-bins`, so test binaries never receive it and die at loader
//! init with STATUS_ENTRYPOINT_NOT_FOUND — `TaskDialogIndirect` is not exported
//! by the v5 comctl32 that an undeclared dependency resolves to. Emit the same
//! manifest as a linker input for the remaining targets; content is identical
//! to the default resource manifest, so the packaged exe is unaffected.
fn main() {
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
