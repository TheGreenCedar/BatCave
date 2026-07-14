#[cfg(windows)]
fn main() {
    std::process::exit(batcave_monitor_lib::run_collector_service());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BatCave Collector Service is available only on Windows.");
    std::process::exit(2);
}
