use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=BATCAVE_INSTALLED_RELEASE");
    println!("cargo:rerun-if-env-changed=BATCAVE_SOURCE_COMMIT_SHA");
    let source_commit_sha = source_commit_sha();
    println!("cargo:rustc-env=BATCAVE_SOURCE_COMMIT_SHA={source_commit_sha}");

    let mut attributes = tauri_build::Attributes::new();
    if cfg!(windows) && std::env::var("BATCAVE_INSTALLED_RELEASE").as_deref() == Ok("1") {
        attributes = attributes.windows_attributes(
            tauri_build::WindowsAttributes::new()
                .app_manifest(include_str!("release.manifest.xml")),
        );
    }
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}

fn source_commit_sha() -> String {
    if let Ok(value) = env::var("BATCAVE_SOURCE_COMMIT_SHA") {
        let value = value.trim();
        assert!(
            valid_source_commit_sha(value),
            "BATCAVE_SOURCE_COMMIT_SHA must be an exact 40-character Git SHA-1"
        );
        return value.to_string();
    }
    String::new()
}

fn valid_source_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
