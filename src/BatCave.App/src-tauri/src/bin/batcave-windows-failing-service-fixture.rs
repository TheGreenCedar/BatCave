#[cfg(windows)]
const EXECUTION_MARKER_NAME: &str = "batcave-rollback-fixture-ran.v1";
#[cfg(windows)]
const EXECUTION_MARKER_BYTES: &[u8] = b"batcave_windows_lifecycle_rollback_fixture_v1\n";

#[cfg(windows)]
fn main() {
    if let Some(exit_code) = fixed_start_failure(std::env::args_os().len()) {
        if let Err(error) = write_execution_marker() {
            eprintln!("{error}");
            std::process::exit(71);
        }
        std::process::exit(exit_code);
    }
    std::process::exit(batcave_monitor_lib::run_collector_service());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The Windows lifecycle service fixture is available only on Windows.");
    std::process::exit(2);
}

fn fixed_start_failure(argument_count: usize) -> Option<i32> {
    (argument_count == 1).then_some(70)
}

#[cfg(windows)]
fn write_execution_marker() -> Result<(), String> {
    use std::io::Write;

    let current = std::env::current_exe()
        .map_err(|error| format!("rollback_fixture_current_exe_failed:{error}"))?;
    let install_directory = current
        .parent()
        .ok_or_else(|| "rollback_fixture_install_directory_missing".to_string())?;
    let path = install_directory.join(EXECUTION_MARKER_NAME);
    let mut marker = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("rollback_fixture_marker_create_failed:{error}"))?;
    marker
        .write_all(EXECUTION_MARKER_BYTES)
        .and_then(|_| marker.sync_all())
        .map_err(|error| format!("rollback_fixture_marker_write_failed:{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_argument_free_scm_start_path_fails_with_the_fixed_code() {
        assert_eq!(fixed_start_failure(1), Some(70));
        assert_eq!(fixed_start_failure(2), None);
        assert_eq!(fixed_start_failure(3), None);
    }
}
