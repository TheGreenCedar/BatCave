#[cfg(windows)]
fn main() {
    if let Some(exit_code) = fixed_start_failure(std::env::args_os().len()) {
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
