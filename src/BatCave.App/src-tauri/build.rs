fn main() {
    println!("cargo:rerun-if-env-changed=BATCAVE_INSTALLED_RELEASE");
    let mut attributes = tauri_build::Attributes::new();
    if cfg!(windows) && std::env::var("BATCAVE_INSTALLED_RELEASE").as_deref() == Ok("1") {
        attributes = attributes.windows_attributes(
            tauri_build::WindowsAttributes::new()
                .app_manifest(include_str!("release.manifest.xml")),
        );
    }
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}
