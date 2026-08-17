#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use crate::app::RivuletApp;
use eframe::egui::{self, IconData};
use rivulet_core::RivuletEngine;
// Tokio is no longer required here, but we keep it for potential future tasks.
use tokio::runtime::Runtime;

/// Point GStreamer at the bundled runtime that ships next to the executable
/// (portable ZIP / MSI). On developer machines (where GStreamer is installed
/// globally) there is no `gstreamer-1.0` folder next to the exe and this is a
/// no-op.
#[cfg(target_os = "windows")]
fn configure_bundled_gstreamer() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let plugin_dir = dir.join("gstreamer-1.0");
            if plugin_dir.is_dir() {
                std::env::set_var("GST_PLUGIN_PATH", &plugin_dir);
                std::env::set_var("GST_PLUGIN_SYSTEM_PATH", &plugin_dir);
                let scanner = dir.join("gst-plugin-scanner.exe");
                if scanner.exists() {
                    std::env::set_var("GST_PLUGIN_SCANNER", scanner);
                }
                let registry = std::env::temp_dir().join("rivulet-gst-registry.bin");
                std::env::set_var("GST_REGISTRY", registry);
            }
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    #[cfg(target_os = "windows")]
    configure_bundled_gstreamer();

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
            // The call is simple again; the app manages its own communication internally.
            let app = RivuletApp::new(cc, engine);
            Ok(Box::new(app))
        }),
    )
}

fn create_icon() -> IconData {
    let ico_bytes = include_bytes!("../assets/rivulet_logo.ico");
    let img = image::load_from_memory(ico_bytes).expect("failed to decode embedded icon");
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}
