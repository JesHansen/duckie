#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(feature = "bench")]
mod bench;
mod bulk;
mod dialogs;
mod editor;
mod shortcuts;
mod state;
mod ui;

/// Straight-alpha pixels for the window icon, which is what the taskbar shows while Duckie runs.
/// A pinned shortcut uses the icon the build script compiled into the executable instead.
///
/// Kept as raw pixels rather than a PNG so the binary needs no image decoder at startup; the size
/// is fixed by `scripts/make-icon.ps1`, and asserted here so the two cannot drift apart.
fn window_icon() -> eframe::egui::IconData {
    const SIDE: usize = 128;
    let rgba = include_bytes!("../../../assets/window-icon.rgba");
    assert_eq!(
        rgba.len(),
        SIDE * SIDE * 4,
        "window-icon.rgba is not 128x128"
    );
    eframe::egui::IconData {
        rgba: rgba.to_vec(),
        width: SIDE as u32,
        height: SIDE as u32,
    }
}
fn main() {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Duckie")
            .with_icon(window_icon())
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    if let Err(error) = eframe::run_native(
        "Duckie",
        options,
        Box::new(|cc| Ok(Box::new(state::Duckie::new(cc)))),
    ) {
        // A release build is a Windows subsystem binary with no console attached, so returning
        // this from main would print it to a stderr nobody is reading and exit silently.
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Duckie could not start")
            .set_description(format!(
                "{error}\n\nDuckie draws with OpenGL 3 or newer. A remote or virtual desktop \
                 session without graphics acceleration may offer only the generic Windows \
                 OpenGL 1.1 driver, which is not enough."
            ))
            .show();
        std::process::exit(1);
    }
}
