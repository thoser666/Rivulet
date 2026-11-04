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
    embed_windows_resources();

    // Copy the Vulkan layer manifest (JSON) to the workspace target directory
    // so VulkanLayerConfig::find() can locate it next to the built layer DLL.
    copy_layer_manifest();

    // Windows: colocate WebView2Loader.dll with the GUI executable so the
    // browser-source adapter's import resolves at startup (a plain cargo
    // build otherwise produces a binary that dies silently before the first
    // log line — Windows never searches cargo's build-script output for
    // import DLLs).
    #[cfg(target_os = "windows")]
    copy_webview2_loader();
}

// Windows: the resource file is generated here instead of by
// `winres::WindowsResource::new()` + `set_icon`, because winres keeps the
// version-info fields and strings in `HashMap`s and writes them in iteration
// order -- which is randomized per process. Two builds of the same source
// therefore produced different `StringFileInfo` ordering and different bytes
// (measured with the /Brepro /DEBUG:NONE flags already in place, issue #288).
// Writing the same fields in a fixed order removes the only nondeterministic
// step in the Windows build without changing what Explorer shows.
#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    use std::fmt::Write as _;

    let name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME");
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    // VERSIONINFO's numeric fields are four 16-bit values, so the pre-release
    // suffix (`-alpha.55`) cannot be represented there; it stays in the string
    // fields, exactly as winres produced it.
    let mut numbers = [0u16; 4];
    for (index, part) in version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .take(4)
        .enumerate()
    {
        numbers[index] = part.parse().unwrap_or(0);
    }

    let mut rc = String::new();
    rc.push_str("#pragma code_page(65001)\n");
    rc.push_str("1 VERSIONINFO\n");
    writeln!(
        rc,
        "FILEVERSION {}, {}, {}, {}",
        numbers[0], numbers[1], numbers[2], numbers[3]
    )
    .expect("write to a String");
    writeln!(
        rc,
        "PRODUCTVERSION {}, {}, {}, {}",
        numbers[0], numbers[1], numbers[2], numbers[3]
    )
    .expect("write to a String");
    rc.push_str("FILEOS 0x40004\n");
    rc.push_str("FILETYPE 0x1\n");
    rc.push_str("FILESUBTYPE 0x0\n");
    rc.push_str("FILEFLAGSMASK 0x3f\n");
    rc.push_str("FILEFLAGS 0x0\n");
    rc.push_str("{\nBLOCK \"StringFileInfo\"\n{\nBLOCK \"000004b0\"\n{\n");
    writeln!(rc, "VALUE \"FileVersion\", \"{version}\"").expect("write to a String");
    writeln!(rc, "VALUE \"ProductVersion\", \"{version}\"").expect("write to a String");
    writeln!(rc, "VALUE \"ProductName\", \"{name}\"").expect("write to a String");
    writeln!(rc, "VALUE \"FileDescription\", \"{name}\"").expect("write to a String");
    rc.push_str("}\n}\nBLOCK \"VarFileInfo\" { VALUE \"Translation\", 0x0, 0x04b0 }\n}\n");
    rc.push_str("1 ICON \"assets/rivulet_logo.ico\"\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let rc_path = std::path::Path::new(&out_dir).join("rivulet.rc");
    std::fs::write(&rc_path, rc).expect("failed to write the Windows resource file");
    println!("cargo:rerun-if-changed=assets/rivulet_logo.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    let mut res = winres::WindowsResource::new();
    res.set_resource_file(rc_path.to_str().expect("OUT_DIR must be valid UTF-8"));
    res.compile().expect("failed to compile Windows resource");
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

#[cfg(target_os = "windows")]
fn copy_webview2_loader() {
    use std::path::PathBuf;

    const LOADER_NAME: &str = "WebView2Loader.dll";

    // The x64 loader ships inside webview2-com-sys's build output. The
    // package directory is hyphenated there (`webview2-com-sys-<hash>`), so
    // the file is located by prefix search over the shared build directory
    // (this script's OUT_DIR is `target/<profile>/build/<pkg>-<hash>/out`,
    // so its grandparent is the shared `target/<profile>/build`).
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let Some(build_root) = out_dir.ancestors().nth(2) else {
        eprintln!("cargo:warning=Unexpected OUT_DIR layout; skipping {LOADER_NAME} colocate");
        return;
    };
    let Ok(entries) = std::fs::read_dir(build_root) else {
        eprintln!(
            "cargo:warning=Build root {} unreadable; skipping {LOADER_NAME} colocate",
            build_root.display()
        );
        return;
    };

    let loader_src = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("webview2-com-sys-")
        })
        .map(|e| e.path().join("out").join("x64").join(LOADER_NAME))
        .find(|p| p.exists());

    let Some(src) = loader_src else {
        eprintln!(
            "cargo:warning=WebView2Loader.dll not found under {} — the GUI exe will not start until it is copied next to it (see README Development section)",
            build_root.display()
        );
        return;
    };

    // Same destination contract as the Vulkan manifest: the workspace
    // profile directory, where cargo places the GUI executable.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir.parent().unwrap();
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let dst = workspace.join("target").join(&profile).join(LOADER_NAME);

    match std::fs::copy(&src, &dst) {
        Ok(_) => {
            println!("cargo:warning=Copied {LOADER_NAME} next to the GUI exe");
        }
        Err(e) => {
            eprintln!("cargo:warning=Failed to copy {LOADER_NAME}: {e}");
        }
    }
}
