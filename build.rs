fn main() {
    // Create dist/ placeholder files if they don't exist.
    // The real binaries are placed here by build.ps1 before building the setup binary.
    // This allows `cargo build --bin taskbar-ip` and `cargo check` to work without build.ps1.
    let dist = std::path::Path::new("dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
    }
    for name in &[
        "taskbar-ip-x86.exe",
        "taskbar-ip-x64.exe",
        "uninstall-x86.exe",
        "uninstall-x64.exe",
    ] {
        let path = dist.join(name);
        if !path.exists() {
            let _ = std::fs::write(&path, b"PLACEHOLDER");
        }
    }

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // embed-manifest defaults already include:
        // - Windows 7 through Windows 11 compatibility
        // - Common Controls v6
        // - DPI awareness
        // - UTF-8 code page
        embed_manifest::embed_manifest(embed_manifest::new_manifest("TaskbarIP"))
            .expect("unable to embed manifest file");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=dist/");
}
