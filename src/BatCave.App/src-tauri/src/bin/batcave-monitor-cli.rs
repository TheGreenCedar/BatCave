fn main() {
    let exit_code = batcave_monitor_lib::run_cli_from_env().unwrap_or_else(|| {
        eprintln!("BatCave CLI requires a supported command such as --benchmark.");
        2
    });
    std::process::exit(exit_code);
}
