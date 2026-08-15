#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use crate::app::RivuletApp;
use eframe::egui::{self, IconData};
use rivulet_core::RivuletEngine;
// Tokio wird hier nicht mehr benötigt, aber wir lassen es für potenzielle andere Aufgaben.
use tokio::runtime::Runtime;

fn main() -> Result<(), eframe::Error> {
    tracing_subscriber::fmt::init();

    let _rt = Runtime::new().unwrap();
    let engine = RivuletEngine::new();

    run_native("Rivulet", engine)
}

fn run_native(app_name: &str, engine: RivuletEngine) -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(800.0, 600.0))
            .with_title(app_name)
            .with_icon(create_icon())
            .with_app_id("rivulet_main_window"),
        ..Default::default()
    };

    eframe::run_native(
        app_name,
        native_options,
        Box::new(|cc| {
            // Der Aufruf ist wieder einfach, die App verwaltet ihre Kommunikation intern.
            let app = RivuletApp::new(cc, engine);
            Ok(Box::new(app))
        }),
    )
}

fn create_icon() -> IconData {
    let image = image::RgbaImage::from_fn(64, 64, |_x, _y| image::Rgba([33, 150, 243, 255]));
    IconData {
        rgba: image.into_raw(),
        width: 64,
        height: 64,
    }
}
