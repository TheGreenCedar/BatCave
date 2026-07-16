#[cfg(windows)]
fn main() {
    std::process::exit(batcave_monitor_lib::run_windows_lifecycle_proof());
}

#[cfg(not(windows))]
fn main() {
    println!(
        "{}",
        serde_json::json!({
            "disposition": "unsupported",
            "reason": "windows_lifecycle_proof_requires_windows"
        })
    );
    std::process::exit(2);
}
