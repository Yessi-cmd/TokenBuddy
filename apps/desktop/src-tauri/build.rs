//! Generates the Tauri context (icons, capabilities, config) at build time.
//!
//! On Windows the Common Controls v6 manifest is normally embedded by
//! tauri-build, but only into `bin` targets: tauri-winres emits
//! `rustc-link-arg-bins`, so library test binaries never receive it and die at
//! loader init with STATUS_ENTRYPOINT_NOT_FOUND — `TaskDialogIndirect` is not
//! exported by the v5 comctl32 that an undeclared dependency resolves to.
//!
//! Disable only tauri-build's manifest resource while retaining its icon and
//! version resources, then provide the same manifest as a linker input for all
//! supported targets. This gives production binaries and test binaries exactly
//! one manifest each.
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().windows_attributes(
            tauri_build::WindowsAttributes::new_without_app_manifest(),
        ),
    )
    .expect("failed to run Tauri build script");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
