#[cfg(target_os = "linux")]
#[path = "batcave-install-smoke/install_smoke_linux.rs"]
mod install_smoke_linux;
#[path = "batcave-install-smoke/install_smoke_release.rs"]
mod install_smoke_release;

fn main() {
    let selectors = std::env::args().skip(1).collect::<Vec<_>>();
    let (outcome, exit_code) = install_smoke_release::run(&selectors);
    println!(
        "{}",
        serde_json::to_string(&outcome).expect("sanitized outcome is serializable")
    );
    std::process::exit(exit_code);
}
