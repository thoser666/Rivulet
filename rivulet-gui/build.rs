fn main() {
    // Linux: force the XCB linker path for the screen capture backend.
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        println!("cargo:rustc-link-lib=xcb");
    }

    // Windows: embed rivulet_logo.ico as the application icon so it shows in
    // Explorer, taskbar, ALT-TAB, etc.
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/rivulet_logo.ico");
        res.compile().expect("failed to compile Windows resource");
    }

    // Copy the Vulkan layer manifest (JSON) to the workspace target directory
    // so VulkanLayerConfig::find() can locate it next to the built layer DLL.
    copy_layer_manifest();
}

fn copy_layer_manifest() {
    let manifest_name = "VkLayer_rivulet_capture.json";
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("rivulet-vulkan-layer")
        .join(manifest_name);

    if !src.exists() {
        eprintln!(
            "cargo:warning=Vulkan layer manifest not found at {}",
            src.display()
        );
        return;
    }

    // Copy to workspace target/{debug,release}/ so the DLL + manifest are colocated
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir.parent().unwrap();
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target_dir = workspace.join("target").join(&profile);

    let dst = target_dir.join(manifest_name);
    if dst.exists() {
        // Already present — skip copy
        return;
    }

    match std::fs::copy(&src, &dst) {
        Ok(_) => {
            println!(
                "cargo:warning=Copied {} to {}",
                manifest_name,
                dst.display()
            );
        }
        Err(e) => {
            eprintln!("cargo:warning=Failed to copy {}: {}", manifest_name, e);
        }
    }
}
