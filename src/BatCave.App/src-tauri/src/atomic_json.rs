use std::{fs, path::Path};

use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub(crate) struct AtomicJsonErrorLabels {
    pub write_failed: &'static str,
    pub serialize_failed: &'static str,
    pub replace_failed: &'static str,
    pub rename_failed: &'static str,
    pub serialize_error_includes_path: bool,
}

pub(crate) fn write_json_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    labels: AtomicJsonErrorLabels,
) -> Result<(), String> {
    write_json_atomic_with_replacer(path, value, labels, replace_file)
}

fn write_json_atomic_with_replacer<T: Serialize>(
    path: &Path,
    value: &T,
    labels: AtomicJsonErrorLabels,
    replacer: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!(
            "{} path={} error=MissingParent",
            labels.write_failed,
            path.display()
        ));
    };
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "{} path={} error={}",
            labels.write_failed,
            parent.display(),
            error
        )
    })?;

    let temp_path = json_temp_path(path);
    let payload = serde_json::to_string(value).map_err(|error| {
        if labels.serialize_error_includes_path {
            format!(
                "{} path={} error={}",
                labels.serialize_failed,
                path.display(),
                error
            )
        } else {
            format!("{}:{error}", labels.serialize_failed)
        }
    })?;

    fs::write(&temp_path, payload).map_err(|error| {
        format!(
            "{} path={} error={}",
            labels.write_failed,
            temp_path.display(),
            error
        )
    })?;
    replacer(&temp_path, path).map_err(|error| {
        let (label, error) = error
            .strip_prefix("rename_failed:")
            .map(|error| (labels.rename_failed, error))
            .unwrap_or((labels.replace_failed, error.as_str()));
        format!("{} path={} error={}", label, path.display(), error)
    })
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, target_path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
    };

    let mut temp_wide = temp_path.as_os_str().encode_wide().collect::<Vec<_>>();
    temp_wide.push(0);
    let mut target_wide = target_path.as_os_str().encode_wide().collect::<Vec<_>>();
    target_wide.push(0);

    let moved = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = unsafe { GetLastError() };
        return Err(format!("MoveFileExW error={error}"));
    }

    Ok(())
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, target_path: &Path) -> Result<(), String> {
    fs::rename(temp_path, target_path).map_err(|error| format!("rename_failed:{error}"))
}

fn json_temp_path(path: &Path) -> std::path::PathBuf {
    path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_LABELS: AtomicJsonErrorLabels = AtomicJsonErrorLabels {
        write_failed: "test_write_failed",
        serialize_failed: "test_serialize_failed",
        replace_failed: "test_replace_failed",
        rename_failed: "test_rename_failed",
        serialize_error_includes_path: true,
    };

    #[test]
    fn writes_json_through_matching_temp_path() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-{}-settings.json",
            std::process::id()
        ));
        let temp_path = json_temp_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&temp_path);

        write_json_atomic(&path, &serde_json::json!({ "ok": true }), TEST_LABELS)
            .expect("json writes");

        let payload = fs::read_to_string(&path).expect("json file exists");
        assert_eq!(payload, r#"{"ok":true}"#);
        assert!(!temp_path.exists());

        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn overwrites_existing_json_without_deleting_first() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-overwrite-{}-settings.json",
            std::process::id()
        ));
        let temp_path = json_temp_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&temp_path);
        fs::write(&path, r#"{"old":true}"#).expect("old json fixture writes");

        write_json_atomic(&path, &serde_json::json!({ "new": true }), TEST_LABELS)
            .expect("json overwrites");

        let payload = fs::read_to_string(&path).expect("json file exists");
        assert_eq!(payload, r#"{"new":true}"#);
        assert!(!temp_path.exists());

        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn replace_failure_preserves_existing_json() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-preserve-{}-settings.json",
            std::process::id()
        ));
        let temp_path = json_temp_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&temp_path);
        fs::write(&path, r#"{"old":true}"#).expect("old json fixture writes");

        let error = write_json_atomic_with_replacer(
            &path,
            &serde_json::json!({ "new": true }),
            TEST_LABELS,
            |_temp, _target| Err("forced_replace_failure".to_string()),
        )
        .expect_err("replace failure is surfaced");

        assert!(error.contains("forced_replace_failure"));
        let payload = fs::read_to_string(&path).expect("json file remains");
        assert_eq!(payload, r#"{"old":true}"#);

        let _ = fs::remove_file(&temp_path);
        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn create_dir_error_uses_write_label() {
        let blocked_parent = std::env::temp_dir().join(format!(
            "batcave-atomic-json-blocked-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&blocked_parent);
        fs::write(&blocked_parent, "not a directory").expect("blocked parent fixture writes");
        let path = blocked_parent.join("settings.json");

        let error = write_json_atomic(&path, &serde_json::json!({ "ok": true }), TEST_LABELS)
            .expect_err("parent create_dir fails");

        fs::remove_file(&blocked_parent).expect("blocked parent cleanup");
        assert!(error.starts_with("test_write_failed path="));
    }
}
