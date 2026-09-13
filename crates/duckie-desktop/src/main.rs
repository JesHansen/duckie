#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(feature = "bench")]
mod bench;
mod dialogs;
mod editor;
mod state;
mod ui;

fn main() {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Duckie")
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
