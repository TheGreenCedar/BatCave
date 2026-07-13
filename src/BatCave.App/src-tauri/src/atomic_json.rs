use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicWriteOperation {
    CreateDirectory,
    SetPermissions,
    CreateTemporary,
    Write,
    SyncFile,
    #[cfg_attr(
        not(windows),
        allow(dead_code, reason = "MoveFileEx replacement is Windows-only")
    )]
    Replace,
    Rename,
    SyncDirectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicWriteEffect {
    NotCommitted,
    CommittedDurabilityUncertain,
}

#[derive(Debug)]
pub(crate) struct AtomicWriteError {
    pub operation: AtomicWriteOperation,
    pub effect: AtomicWriteEffect,
    pub path: PathBuf,
    pub error: io::Error,
}

impl AtomicWriteError {
    fn new(operation: AtomicWriteOperation, path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self {
            operation,
            effect: AtomicWriteEffect::NotCommitted,
            path: path.into(),
            error,
        }
    }

    fn committed_durability_uncertain(
        operation: AtomicWriteOperation,
        path: impl Into<PathBuf>,
        error: io::Error,
    ) -> Self {
        Self {
            operation,
            effect: AtomicWriteEffect::CommittedDurabilityUncertain,
            path: path.into(),
            error,
        }
    }
}

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
    let payload = serde_json::to_vec(value).map_err(|error| {
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
    write_bytes_atomic(path, &payload).map_err(|error| {
        let label = match error.operation {
            AtomicWriteOperation::Replace => labels.replace_failed,
            AtomicWriteOperation::Rename | AtomicWriteOperation::SyncDirectory => {
                labels.rename_failed
            }
            AtomicWriteOperation::CreateDirectory
            | AtomicWriteOperation::SetPermissions
            | AtomicWriteOperation::CreateTemporary
            | AtomicWriteOperation::Write
            | AtomicWriteOperation::SyncFile => labels.write_failed,
        };
        format!(
            "{} path={} error={}",
            label,
            error.path.display(),
            error.error
        )
    })
}

pub(crate) fn write_bytes_atomic(path: &Path, payload: &[u8]) -> Result<(), AtomicWriteError> {
    write_bytes_atomic_with_replacer(path, payload, replace_file)
}

fn write_bytes_atomic_with_replacer(
    path: &Path,
    payload: &[u8],
    replacer: impl FnOnce(&Path, &Path) -> Result<(), AtomicWriteError>,
) -> Result<(), AtomicWriteError> {
    let parent = path.parent().ok_or_else(|| {
        AtomicWriteError::new(
            AtomicWriteOperation::CreateDirectory,
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory"),
        )
    })?;
    #[cfg(unix)]
    let parent_was_created = !parent.exists();
    fs::create_dir_all(parent).map_err(|error| {
        AtomicWriteError::new(AtomicWriteOperation::CreateDirectory, parent, error)
    })?;
    #[cfg(unix)]
    if parent_was_created {
        fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700)).map_err(
            |error| AtomicWriteError::new(AtomicWriteOperation::SetPermissions, parent, error),
        )?;
    }

    let (temp_path, mut temp_file) = create_temp_file(path)?;
    if let Err(error) = temp_file.write_all(payload) {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(AtomicWriteError::new(
            AtomicWriteOperation::Write,
            temp_path,
            error,
        ));
    }
    if let Err(error) = temp_file.sync_all() {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(AtomicWriteError::new(
            AtomicWriteOperation::SyncFile,
            temp_path,
            error,
        ));
    }
    drop(temp_file);
    if let Err(error) = replacer(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
fn write_json_atomic_with_replacer<T: Serialize>(
    path: &Path,
    value: &T,
    labels: AtomicJsonErrorLabels,
    replacer: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let payload = serde_json::to_vec(value).map_err(|error| {
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
    write_bytes_atomic_with_replacer(path, &payload, |temp_path, target_path| {
        replacer(temp_path, target_path).map_err(|detail| {
            let (operation, detail) = detail
                .strip_prefix("rename_failed:")
                .map(|detail| (AtomicWriteOperation::Rename, detail))
                .unwrap_or((AtomicWriteOperation::Replace, detail.as_str()));
            AtomicWriteError::new(operation, target_path, io::Error::other(detail.to_string()))
        })
    })
    .map_err(|error| {
        let label = match error.operation {
            AtomicWriteOperation::Replace => labels.replace_failed,
            AtomicWriteOperation::Rename | AtomicWriteOperation::SyncDirectory => {
                labels.rename_failed
            }
            _ => labels.write_failed,
        };
        format!(
            "{} path={} error={}",
            label,
            error.path.display(),
            error.error
        )
    })
}

fn create_temp_file(path: &Path) -> Result<(PathBuf, File), AtomicWriteError> {
    let parent = path.parent().expect("parent checked before temp creation");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data.json");
    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!("{name}.{}.{}.tmp", std::process::id(), sequence));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&temp_path) {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(AtomicWriteError::new(
                    AtomicWriteOperation::CreateTemporary,
                    temp_path,
                    error,
                ));
            }
        }
    }
    Err(AtomicWriteError::new(
        AtomicWriteOperation::CreateTemporary,
        path,
        io::Error::new(io::ErrorKind::AlreadyExists, "temporary name exhausted"),
    ))
}

#[cfg(windows)]
fn replace_file(temp_path: &Path, target_path: &Path) -> Result<(), AtomicWriteError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION},
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
    };

    let mut temp_wide = temp_path.as_os_str().encode_wide().collect::<Vec<_>>();
    temp_wide.push(0);
    let mut target_wide = target_path.as_os_str().encode_wide().collect::<Vec<_>>();
    target_wide.push(0);

    for attempt in 0..50 {
        let moved = unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved != 0 {
            return Ok(());
        }
        let error = unsafe { GetLastError() };
        if attempt < 49 && matches!(error, ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION) {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        return Err(AtomicWriteError::new(
            AtomicWriteOperation::Replace,
            target_path,
            io::Error::from_raw_os_error(error as i32),
        ));
    }
    unreachable!()
}

#[cfg(not(windows))]
fn replace_file(temp_path: &Path, target_path: &Path) -> Result<(), AtomicWriteError> {
    fs::rename(temp_path, target_path)
        .map_err(|error| AtomicWriteError::new(AtomicWriteOperation::Rename, target_path, error))?;
    let parent = target_path.parent().ok_or_else(|| {
        AtomicWriteError::new(
            AtomicWriteOperation::SyncDirectory,
            target_path,
            io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory"),
        )
    })?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            AtomicWriteError::committed_durability_uncertain(
                AtomicWriteOperation::SyncDirectory,
                target_path,
                error,
            )
        })
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
    fn writes_json_through_unique_temp_path() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-{}-settings.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        remove_temp_files(&path);

        write_json_atomic(&path, &serde_json::json!({ "ok": true }), TEST_LABELS)
            .expect("json writes");

        let payload = fs::read_to_string(&path).expect("json file exists");
        assert_eq!(payload, r#"{"ok":true}"#);
        assert!(temp_files(&path).is_empty());

        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn overwrites_existing_json_without_deleting_first() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-overwrite-{}-settings.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        remove_temp_files(&path);
        fs::write(&path, r#"{"old":true}"#).expect("old json fixture writes");

        write_json_atomic(&path, &serde_json::json!({ "new": true }), TEST_LABELS)
            .expect("json overwrites");

        let payload = fs::read_to_string(&path).expect("json file exists");
        assert_eq!(payload, r#"{"new":true}"#);
        assert!(temp_files(&path).is_empty());

        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn replace_failure_preserves_existing_json() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-preserve-{}-settings.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        remove_temp_files(&path);
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

        assert!(temp_files(&path).is_empty());
        fs::remove_file(&path).expect("json cleanup");
    }

    #[cfg(not(windows))]
    #[test]
    fn post_commit_directory_sync_failure_reports_installed_target() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-post-commit-{}-settings.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        remove_temp_files(&path);
        fs::write(&path, r#"{"old":true}"#).expect("old json fixture writes");

        let error = write_bytes_atomic_with_replacer(&path, br#"{"new":true}"#, |temp, target| {
            fs::rename(temp, target).expect("replacement commits");
            Err(AtomicWriteError::committed_durability_uncertain(
                AtomicWriteOperation::SyncDirectory,
                target,
                io::Error::new(
                    io::ErrorKind::ReadOnlyFilesystem,
                    "forced directory sync failure",
                ),
            ))
        })
        .expect_err("directory sync failure is surfaced");

        assert_eq!(
            error.effect,
            AtomicWriteEffect::CommittedDurabilityUncertain
        );
        assert_eq!(
            fs::read_to_string(&path).expect("new target remains installed"),
            r#"{"new":true}"#
        );
        assert!(temp_files(&path).is_empty());
        fs::remove_file(&path).expect("json cleanup");
    }

    #[test]
    fn concurrent_writers_never_share_temp_files() {
        let path = std::env::temp_dir().join(format!(
            "batcave-atomic-json-concurrent-{}-settings.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        remove_temp_files(&path);

        let writers = (0..8)
            .map(|writer| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for value in 0..20 {
                        write_json_atomic(
                            &path,
                            &serde_json::json!({ "writer": writer, "value": value }),
                            TEST_LABELS,
                        )?;
                    }
                    Ok::<_, String>(())
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer
                .join()
                .expect("writer joins")
                .expect("writer succeeds");
        }

        let payload = fs::read_to_string(&path).expect("json file exists");
        serde_json::from_str::<serde_json::Value>(&payload).expect("final payload is valid json");
        assert!(temp_files(&path).is_empty());
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

    #[cfg(unix)]
    #[test]
    fn existing_parent_permissions_are_preserved() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let parent = std::env::temp_dir().join(format!(
            "batcave-atomic-json-permissions-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir(&parent).expect("owned parent fixture");
        fs::set_permissions(&parent, PermissionsExt::from_mode(0o755))
            .expect("fixture permissions");

        write_json_atomic(
            &parent.join("settings.json"),
            &serde_json::json!({ "ok": true }),
            TEST_LABELS,
        )
        .expect("json writes");

        assert_eq!(fs::metadata(&parent).unwrap().mode() & 0o777, 0o755);
        fs::remove_dir_all(parent).expect("fixture cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_parent_is_private() {
        use std::os::unix::fs::MetadataExt;

        let root = std::env::temp_dir().join(format!(
            "batcave-atomic-json-private-{}",
            std::process::id()
        ));
        let parent = root.join("owned-child");
        let _ = fs::remove_dir_all(&root);

        write_json_atomic(
            &parent.join("settings.json"),
            &serde_json::json!({ "ok": true }),
            TEST_LABELS,
        )
        .expect("json writes");

        assert_eq!(fs::metadata(&parent).unwrap().mode() & 0o777, 0o700);
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    fn temp_files(path: &Path) -> Vec<PathBuf> {
        let Some(parent) = path.parent() else {
            return Vec::new();
        };
        let prefix = format!("{}.", path.file_name().unwrap().to_string_lossy());
        fs::read_dir(parent)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
            })
            .collect()
    }

    fn remove_temp_files(path: &Path) {
        for temp in temp_files(path) {
            let _ = fs::remove_file(temp);
        }
    }
}
