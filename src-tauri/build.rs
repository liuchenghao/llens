fn main() {
    tauri_build::build();
    // Link CoreGraphics for CGDisplayIsAsleep (display-asleep detection).
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
    }
}
