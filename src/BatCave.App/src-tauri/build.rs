use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    println!("cargo:rerun-if-env-changed=BATCAVE_SOURCE_COMMIT_SHA");
    let source_commit_sha = source_commit_sha();
    println!("cargo:rustc-env=BATCAVE_SOURCE_COMMIT_SHA={source_commit_sha}");

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS must be set");
    if target_os == "macos" {
        build_macos_foundation_models_sidecar();
    }
    if matches!(target_os.as_str(), "windows" | "linux") {
        stage_foundry_native_libraries(&target_os);
    }

    let mut attributes = tauri_build::Attributes::new();
    if target_os == "windows" {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_windows_manifest();
    }
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}

fn stage_foundry_native_libraries(target_os: &str) {
    let expected_files: &[&str] = match target_os {
        "windows" => &[
            "Microsoft.AI.Foundry.Local.Core.dll",
            "onnxruntime.dll",
            "onnxruntime-genai.dll",
        ],
        "linux" => &[
            "Microsoft.AI.Foundry.Local.Core.so",
            "libonnxruntime.so",
            "libonnxruntime-genai.so",
        ],
        _ => return,
    };
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("Cargo.lock").display()
    );

    let own_out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set by Cargo"));
    let build_root = own_out_dir
        .parent()
        .and_then(Path::parent)
        .expect("OUT_DIR must be nested under Cargo's build directory");
    let source = newest_foundry_native_dir(build_root, expected_files).unwrap_or_else(|| {
        panic!(
            "foundry-local-sdk 1.2.0 native libraries were not found under {}",
            build_root.display()
        )
    });
    let destination = manifest_dir.join(".generated/foundry-native");
    if destination.exists() {
        fs::remove_dir_all(&destination).expect("failed to clear staged Foundry native libraries");
    }
    fs::create_dir_all(&destination).expect("failed to create Foundry native staging directory");
    for file_name in expected_files {
        let input = source.join(file_name);
        let metadata = fs::metadata(&input).expect("staged Foundry native library is missing");
        assert!(
            metadata.is_file() && metadata.len() > 0,
            "staged Foundry native library must be a non-empty file: {}",
            input.display()
        );
        fs::copy(&input, destination.join(file_name))
            .expect("failed to stage Foundry native library");
    }
}

fn newest_foundry_native_dir(build_root: &Path, expected_files: &[&str]) -> Option<PathBuf> {
    fs::read_dir(build_root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("foundry-local-sdk-")
        })
        .map(|entry| entry.path().join("out"))
        .filter(|candidate| {
            expected_files
                .iter()
                .all(|file_name| candidate.join(file_name).is_file())
        })
        .max_by_key(|candidate| {
            expected_files
                .iter()
                .filter_map(|file_name| fs::metadata(candidate.join(file_name)).ok())
                .filter_map(|metadata| metadata.modified().ok())
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH)
        })
}

fn build_macos_foundation_models_sidecar() {
    const SIDECAR_NAME: &str = "batcave-foundation-models";
    const MACOS_DEPLOYMENT_TARGET: &str = "12.0";

    let target = env::var("TARGET").expect("TARGET must be set by Cargo");
    assert_eq!(
        target, "aarch64-apple-darwin",
        "the Foundation Models sidecar supports Apple Silicon macOS only"
    );
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    let source_dir = manifest_dir.join("swift/foundation-models-sidecar");
    let protocol_source = source_dir.join("SidecarProtocol.swift");
    let executable_source = source_dir.join("FoundationModelsSidecar.swift");
    println!("cargo:rerun-if-changed={}", protocol_source.display());
    println!("cargo:rerun-if-changed={}", executable_source.display());
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=SDKROOT");

    let output_dir = manifest_dir.join("target/foundation-models-sidecar");
    fs::create_dir_all(&output_dir).expect("failed to create Foundation Models sidecar output");
    let output = output_dir.join(format!("{SIDECAR_NAME}-{target}"));
    let sdk = macos_sdk_path();
    let foundation_models_available = sdk
        .join("System/Library/Frameworks/FoundationModels.framework")
        .is_dir();
    let mut compiler = Command::new("xcrun");
    compiler
        .args(["--sdk", "macosx", "swiftc"])
        .arg(&protocol_source)
        .arg(&executable_source)
        .args([
            "-parse-as-library",
            "-module-name",
            "BatCaveFoundationModelsSidecar",
            "-target",
            "arm64-apple-macos12.0",
            "-sdk",
        ])
        .arg(&sdk)
        .args([
            "-O",
            "-whole-module-optimization",
            "-framework",
            "Foundation",
        ]);
    if foundation_models_available {
        compiler.args([
            "-Xlinker",
            "-weak_framework",
            "-Xlinker",
            "FoundationModels",
        ]);
    }
    let compile = compiler
        .arg("-o")
        .arg(&output)
        .env("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET)
        .output()
        .expect("failed to launch the Swift compiler for the Foundation Models sidecar");
    assert!(
        compile.status.success(),
        "Foundation Models sidecar compilation failed:\n{}{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );
}

fn macos_sdk_path() -> PathBuf {
    let output = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .expect("failed to query the macOS SDK path");
    assert!(
        output.status.success(),
        "failed to query the macOS SDK path: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = String::from_utf8(output.stdout).expect("macOS SDK path must be UTF-8");
    PathBuf::from(path.trim())
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
