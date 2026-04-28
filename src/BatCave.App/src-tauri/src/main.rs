fn main() {
    if let Some(exit_code) = batcave_monitor_lib::run_cli_from_env() {
        std::process::exit(exit_code);
    }

    batcave_monitor_lib::run();
}
