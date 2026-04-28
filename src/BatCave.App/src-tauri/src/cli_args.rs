pub(crate) fn reject_unknown_args(
    args: &[String],
    known_with_value: &[&str],
    known_flags: &[&str],
) -> Result<(), String> {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if known_flags.contains(&arg.as_str()) {
            index += 1;
        } else if known_with_value.contains(&arg.as_str()) {
            if index + 1 >= args.len() {
                return Err(format!("missing_value_for_argument:{arg}"));
            }
            if args[index + 1].starts_with("--") {
                return Err(format!("missing_value_for_argument:{arg}"));
            }
            index += 2;
        } else {
            return Err(format!("unknown_argument:{arg}"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_arguments() {
        let args = vec!["--known".to_string(), "--wat".to_string()];

        assert_eq!(
            reject_unknown_args(&args, &[], &["--known"]),
            Err("unknown_argument:--wat".to_string())
        );
    }

    #[test]
    fn rejects_missing_values_before_next_flag() {
        let args = vec!["--value".to_string(), "--flag".to_string()];

        assert_eq!(
            reject_unknown_args(&args, &["--value"], &["--flag"]),
            Err("missing_value_for_argument:--value".to_string())
        );
    }
}
