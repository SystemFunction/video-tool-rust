// Hide the console window in release builds (GUI app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod binaries;
mod config;
mod consts;
mod convert;
mod download;
mod emit;
mod types;
mod util;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1220.0, 840.0])
            .with_min_inner_size([960.0, 700.0])
            .with_title(format!("{} v{}", consts::APP_NAME, consts::VERSION)),
        ..Default::default()
    };
    eframe::run_native(
        &format!("{} v{}", consts::APP_NAME, consts::VERSION),
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
