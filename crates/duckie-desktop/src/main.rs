#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod dialogs;
mod editor;
mod state;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Duckie")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Duckie",
        options,
        Box::new(|cc| Ok(Box::new(state::Duckie::new(cc)))),
    )
}
