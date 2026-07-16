use std::{fs, io, path::Path};

const HELPER_DIRECTORY: &str = "elevated-helper";
const RUN_PREFIX: &str = "run-";
const TOKEN_HEX_LENGTH: usize = 64;
const ARTIFACT_NAMES: [&str; 4] = [
    "snapshot.json",
    "snapshot.json.tmp",
    "stop.signal",
    "accepted.signal",
];

#[cfg(windows)]
pub(crate) fn legacy_state_present(base_dir: &Path) -> Result<bool, String> {
    metadata_if_present(&base_dir.join(HELPER_DIRECTORY), "helper_root")
        .map(|value| value.is_some())
}

pub(crate) fn remove_legacy_artifacts(base_dir: &Path) -> Result<bool, String> {
    let helper_root = base_dir.join(HELPER_DIRECTORY);
    let Some(metadata) = metadata_if_present(&helper_root, "helper_root")? else {
        return Ok(false);
    };
    require_real_directory(&metadata, "helper_root")?;

    let mut removed = remove_known_files(&helper_root, "helper_root")?;
    let entries = fs::read_dir(&helper_root)
        .map_err(|error| migration_error("read", "helper_root", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| migration_error("read", "helper_root", error))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_legacy_run_directory(&name) {
            continue;
        }

        let run_dir = entry.path();
        let metadata = fs::symlink_metadata(&run_dir)
            .map_err(|error| migration_error("metadata", "run_directory", error))?;
        require_real_directory(&metadata, "run_directory")?;
        removed |= remove_known_files(&run_dir, "run_directory")?;
        if directory_is_empty(&run_dir, "run_directory")? {
            fs::remove_dir(&run_dir)
                .map_err(|error| migration_error("remove", "run_directory", error))?;
            removed = true;
        }
    }

    if directory_is_empty(&helper_root, "helper_root")? {
        fs::remove_dir(&helper_root)
            .map_err(|error| migration_error("remove", "helper_root", error))?;
        removed = true;
    }
    Ok(removed)
}

fn remove_known_files(directory: &Path, scope: &str) -> Result<bool, String> {
    let mut existing = Vec::new();
    for name in ARTIFACT_NAMES {
        let path = directory.join(name);
        let Some(metadata) = metadata_if_present(&path, name)? else {
            continue;
        };
        if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
            return Err(format!(
                "legacy_helper_migration_reparse_rejected:{scope}:{name}"
            ));
        }
        if !metadata.is_file() {
            return Err(format!(
                "legacy_helper_migration_type_rejected:{scope}:{name}"
            ));
        }
        existing.push(path);
    }

    for path in &existing {
        fs::remove_file(path).map_err(|error| {
            migration_error(
                "remove",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("artifact"),
                error,
            )
        })?;
    }
    Ok(!existing.is_empty())
}

fn metadata_if_present(path: &Path, scope: &str) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(migration_error("metadata", scope, error)),
    }
}

fn require_real_directory(metadata: &fs::Metadata, scope: &str) -> Result<(), String> {
    if metadata.file_type().is_symlink() || is_windows_reparse_point(metadata) {
        return Err(format!("legacy_helper_migration_reparse_rejected:{scope}"));
    }
    if !metadata.is_dir() {
        return Err(format!("legacy_helper_migration_type_rejected:{scope}"));
    }
    Ok(())
}

fn directory_is_empty(path: &Path, scope: &str) -> Result<bool, String> {
    let mut entries = fs::read_dir(path).map_err(|error| migration_error("read", scope, error))?;
    match entries.next() {
        None => Ok(true),
        Some(Ok(_)) => Ok(false),
        Some(Err(error)) => Err(migration_error("read", scope, error)),
    }
}

fn is_legacy_run_directory(name: &str) -> bool {
    name.strip_prefix(RUN_PREFIX).is_some_and(|token| {
        token.len() == TOKEN_HEX_LENGTH
            && token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn migration_error(operation: &str, scope: &str, error: io::Error) -> String {
    format!(
        "legacy_helper_migration_{operation}_failed:{scope}:{:?}",
        error.kind()
    )
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn removes_root_and_valid_run_artifacts_idempotently() {
        let base_dir = test_dir("owned-artifacts");
        let helper_root = base_dir.join(HELPER_DIRECTORY);
        let run_dir = helper_root.join(format!("{RUN_PREFIX}{}", "a".repeat(TOKEN_HEX_LENGTH)));
        fs::create_dir_all(&run_dir).expect("legacy run directory creates");
        for directory in [&helper_root, &run_dir] {
            for name in ARTIFACT_NAMES {
                fs::write(directory.join(name), "legacy").expect("legacy artifact writes");
            }
        }

        assert_eq!(remove_legacy_artifacts(&base_dir), Ok(true));
        assert!(!helper_root.exists());
        assert_eq!(remove_legacy_artifacts(&base_dir), Ok(false));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn preserves_unknown_entries_and_invalid_run_names() {
        let base_dir = test_dir("unknown-entries");
        let helper_root = base_dir.join(HELPER_DIRECTORY);
        let valid_run = helper_root.join(format!("{RUN_PREFIX}{}", "b".repeat(TOKEN_HEX_LENGTH)));
        let invalid_run = helper_root.join("run-not-a-shipped-token");
        fs::create_dir_all(&valid_run).expect("valid run directory creates");
        fs::create_dir_all(&invalid_run).expect("invalid run directory creates");
        fs::write(helper_root.join("keep.txt"), "keep").expect("unknown root file writes");
        fs::write(valid_run.join("keep.txt"), "keep").expect("unknown run file writes");
        fs::write(valid_run.join("snapshot.json"), "legacy").expect("known artifact writes");
        fs::write(invalid_run.join("snapshot.json"), "keep").expect("invalid run file writes");

        assert_eq!(remove_legacy_artifacts(&base_dir), Ok(true));
        assert!(helper_root.join("keep.txt").exists());
        assert!(valid_run.join("keep.txt").exists());
        assert!(!valid_run.join("snapshot.json").exists());
        assert!(invalid_run.join("snapshot.json").exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn rejects_reparse_run_directories_without_touching_the_target() {
        let base_dir = test_dir("reparse-run");
        let helper_root = base_dir.join(HELPER_DIRECTORY);
        let outside = test_dir("reparse-outside");
        let run_dir = helper_root.join(format!("{RUN_PREFIX}{}", "c".repeat(TOKEN_HEX_LENGTH)));
        fs::create_dir_all(&helper_root).expect("helper root creates");
        fs::create_dir_all(&outside).expect("outside directory creates");
        fs::write(outside.join("snapshot.json"), "outside").expect("outside file writes");
        symlink_dir(&outside, &run_dir).expect("reparse run directory fixture creates");

        assert!(remove_legacy_artifacts(&base_dir)
            .expect_err("reparse run directory is rejected")
            .starts_with("legacy_helper_migration_reparse_rejected:run_directory"));
        assert_eq!(
            fs::read_to_string(outside.join("snapshot.json")).expect("outside file remains"),
            "outside"
        );
        let _ = fs::remove_dir_all(base_dir);
        let _ = fs::remove_dir_all(outside);
    }

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "batcave-legacy-helper-migration-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[cfg(unix)]
    fn symlink_dir(source: &Path, target: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(source, target)
    }

    #[cfg(windows)]
    fn symlink_dir(source: &Path, target: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_dir(source, target)
    }
}
