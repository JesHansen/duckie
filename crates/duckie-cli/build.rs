//! Compiles the CLI icon into the executable so Windows displays its own artwork in Explorer.
//!
//! The icon is generated from `assets/duckie-cli.png` by `scripts/make-icon.ps1` and committed;
//! this build script only embeds it.
fn main() {
    let assets = std::path::Path::new("../../assets");
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        assets.join("duckie-cli.ico").display()
    );
    if !cfg!(target_os = "windows") {
        return;
    }
    let icon = assets.join("duckie-cli.ico");
    if !icon.exists() {
        println!(
            "cargo:warning={} is missing; run scripts/make-icon.ps1 for the CLI",
            icon.display()
        );
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(&icon.to_string_lossy());
    resource.set("FileDescription", "Duckie CLI");
    resource.set("ProductName", "Duckie");
    resource.set(
        "LegalCopyright",
        "Copyright (c) 2026 Jes Bak Hansen. MIT licensed.",
    );
    if let Err(e) = resource.compile() {
        println!("cargo:warning=Could not embed the CLI icon: {e}");
    }
}
