fn main() {
    // Dieses Build-Script läuft nur, wenn für Linux kompiliert wird.
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
        println!("cargo:rustc-link-lib=xcb");
        println!("!!! Rivulet build script is forcing linker path for Linux !!!");
    }
}
