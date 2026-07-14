#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if let Some(exit_code) = batcave_monitor_lib::run_cli_from_env() {
        std::process::exit(exit_code);
    }

    if let Err(error) = batcave_monitor_lib::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
