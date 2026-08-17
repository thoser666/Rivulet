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
}
