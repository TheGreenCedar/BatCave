use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=BATCAVE_SOURCE_COMMIT_SHA");
    let source_commit_sha = source_commit_sha();
    println!("cargo:rustc-env=BATCAVE_SOURCE_COMMIT_SHA={source_commit_sha}");

    let mut attributes = tauri_build::Attributes::new();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_windows_manifest();
    }
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}

fn embed_windows_manifest() {
    let manifest_path = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    )
    .join("release.manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest_path.display());

    // tauri-build links its manifest only to binary targets. Linking the same
    // Common-Controls v6 manifest here also covers Windows test executables;
    // see https://github.com/tauri-apps/tauri/issues/13419.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
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
