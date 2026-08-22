//! Copy the layer manifest (`VkLayer_rivulet_capture.json`) next to the built
//! layer DLL so the Vulkan loader discovers it via `VK_LAYER_PATH=<target dir>`.
//!
//! This makes the layer crate self-contained: `cargo test -p rivulet-vulkan-layer`
//! (and the live smoke test in `tests/smoke.rs`) can enable the layer without
//! also building the GUI (whose build.rs has an equivalent copy step).

fn main() {
    let manifest_name = "VkLayer_rivulet_capture.json";
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(manifest_name);
    if !src.exists() {
        eprintln!(
            "cargo:warning=Vulkan layer manifest not found at {}",
            src.display()
        );
        return;
    }

    // Copy to the workspace target/<profile>/ dir, colocated with the cdylib
    // (`rivulet_vulkan_layer.dll` / `librivulet_vulkan_layer.so`).
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let dst = workspace.join("target").join(&profile).join(manifest_name);

    // Always overwrite so a fixed `library_path` propagates to existing target
    // dirs (a previous copy may predate the underscore fix).
    match std::fs::copy(&src, &dst) {
        Ok(_) => println!(
            "cargo:warning=Copied {} to {}",
            manifest_name,
            dst.display()
        ),
        Err(e) => eprintln!(
            "cargo:warning=Failed to copy {} to {}: {}",
            manifest_name,
            dst.display(),
            e
        ),
    }
}
