#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod camera;
mod paint;
mod render;
mod uv_editor;
mod viewport;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 860.0]).with_title("PixelMesh"),
        depth_buffer: 24,
        ..Default::default()
    };
    // Optional arguments: a .pmesh project, an .fbx to import and/or a texture image to open at startup.
    let args: Vec<String> = std::env::args().skip(1).collect();
    eframe::run_native("PixelMesh", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, &args)))))
}
