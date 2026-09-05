//! A normal installed GUI launch may create one entry in its own user's Start menu.
//! Existing entries are never inspected, adopted, updated, or removed. Users repair an
//! existing entry by removing it and launching the installed app again. Machine uninstall
//! leaves this user state alone, as required by ADR 0013.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserLaunchOutcome {
    Created,
    ExistingEntryPreserved,
    SkippedContext,
}

fn with_user_context(
    elevated: bool,
    session: u32,
    service_account: bool,
    impersonating: bool,
    create: impl FnOnce() -> Result<UserLaunchOutcome, String>,
) -> Result<UserLaunchOutcome, String> {
    if elevated || session == 0 || service_account || impersonating {
        return Ok(UserLaunchOutcome::SkippedContext);
    }
    create()
}

fn is_creation_collision(error: u32) -> bool {
    // ERROR_FILE_EXISTS / ERROR_ALREADY_EXISTS: neither authorizes opening the existing leaf.
    matches!(error, 80 | 183)
}

fn publish_missing_entry<T>(
    create_new: impl FnOnce() -> Result<Option<T>, String>,
    publish: impl FnOnce(T) -> Result<(), String>,
) -> Result<UserLaunchOutcome, String> {
    let Some(entry) = create_new()? else {
        return Ok(UserLaunchOutcome::ExistingEntryPreserved);
    };
    publish(entry)?;
    Ok(UserLaunchOutcome::Created)
}

#[cfg(windows)]
pub(crate) use native::ensure_current_user_start_entry;

#[cfg(windows)]
mod native {
    use std::{
        ffi::OsString,
        fs::File,
        io::Write,
        mem::size_of,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            io::{AsRawHandle, FromRawHandle},
        },
        path::{Path, PathBuf},
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_TOKEN, HANDLE,
        },
        Security::{
            GetTokenInformation, IsWellKnownSid, TokenElevation, TokenSessionId, TokenUser,
            WinLocalServiceSid, WinLocalSystemSid, WinNetworkServiceSid, TOKEN_DUPLICATE,
            TOKEN_ELEVATION, TOKEN_IMPERSONATE, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateFileW, FileDispositionInfo, GetFileInformationByHandle,
            GetFinalPathNameByHandleW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
            CREATE_NEW, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING,
        },
        System::{
            Com::CoTaskMemFree,
            Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
        },
        UI::Shell::{FOLDERID_Programs, SHGetKnownFolderPath},
    };

    use super::{
        is_creation_collision, publish_missing_entry, with_user_context, UserLaunchOutcome,
    };

    const ENTRY_NAME: &str = "BatCave Monitor.lnk";
    const MAX_SHORTCUT_BYTES: usize = 1024 * 1024;

    pub(crate) fn ensure_current_user_start_entry() -> Result<UserLaunchOutcome, String> {
        // A process token is explicit; no active session lookup, linked token, profile guess,
        // or installer account is used to select the destination.
        if thread_is_impersonating()? {
            return Ok(UserLaunchOutcome::SkippedContext);
        }
        let token = process_token()?;
        let elevation: TOKEN_ELEVATION = token_value(token.0, TokenElevation)?;
        let session: u32 = token_value(token.0, TokenSessionId)?;
        with_user_context(
            elevation.TokenIsElevated != 0,
            session,
            service_account(token.0)?,
            false,
            || {
                crate::collector_service::windows_provisioner::with_verified_current_monitor(
                    |monitor| {
                        let root = programs_folder(token.0)?;
                        let ancestry = pin_ancestry(&root)?;
                        let bytes = crate::collector_service::windows_shortcut_retirement::user_launch_shortcut_bytes(monitor)?;
                        if bytes.is_empty() || bytes.len() > MAX_SHORTCUT_BYTES {
                            return Err("user_launch_shortcut_bytes_invalid".to_string());
                        }
                        revalidate_destination(token.0, &root, &ancestry)?;
                        let path = root.join(ENTRY_NAME);
                        publish_missing_entry(
                            || NewEntry::create(&path),
                            |mut entry| {
                                let result = (|| {
                                    entry.file.write_all(&bytes).map_err(|error| {
                                        format!("user_launch_write_failed:{error}")
                                    })?;
                                    entry.file.sync_all().map_err(|error| {
                                        format!("user_launch_flush_failed:{error}")
                                    })?;
                                    entry.validate(&path, bytes.len() as u64)?;
                                    revalidate_destination(token.0, &root, &ancestry)
                                })();
                                match result {
                                    Ok(()) => {
                                        entry.committed = true;
                                        Ok(())
                                    }
                                    Err(primary) => match entry.rollback() {
                                        Ok(()) => Err(primary),
                                        Err(rollback) => Err(format!("{primary};{rollback}")),
                                    },
                                }
                            },
                        )
                    },
                )
            },
        )
    }

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn process_token() -> Result<OwnedHandle, String> {
        let mut raw = ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_DUPLICATE,
                &mut raw,
            )
        } == 0
        {
            return Err(last_error("user_launch_token_open_failed"));
        }
        Ok(OwnedHandle(raw))
    }

    fn thread_is_impersonating() -> Result<bool, String> {
        let mut raw = ptr::null_mut();
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) } != 0 {
            let _token = OwnedHandle(raw);
            return Ok(true);
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_NO_TOKEN {
            Ok(false)
        } else {
            Err(format!("user_launch_thread_token_failed:{error}"))
        }
    }

    fn token_value<T: Default>(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<T, String> {
        let mut value = T::default();
        let mut returned = 0;
        if unsafe {
            GetTokenInformation(
                token,
                class,
                (&mut value as *mut T).cast(),
                size_of::<T>() as u32,
                &mut returned,
            )
        } == 0
            || returned as usize != size_of::<T>()
        {
            return Err(last_error("user_launch_token_query_failed"));
        }
        Ok(value)
    }

    fn service_account(token: HANDLE) -> Result<bool, String> {
        let mut bytes = 0;
        if unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes) } != 0
            || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
            || !(size_of::<TOKEN_USER>()..=4096).contains(&(bytes as usize))
        {
            return Err("user_launch_token_user_size_invalid".to_string());
        }
        // usize storage preserves the alignment required by TOKEN_USER and its SID.
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let capacity = buffer.len() * size_of::<usize>();
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                capacity as u32,
                &mut bytes,
            )
        } == 0
        {
            return Err(last_error("user_launch_token_user_failed"));
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        if user.User.Sid.is_null() {
            return Err("user_launch_token_user_missing".to_string());
        }
        Ok(
            [WinLocalSystemSid, WinLocalServiceSid, WinNetworkServiceSid]
                .into_iter()
                .any(|kind| unsafe { IsWellKnownSid(user.User.Sid, kind) } != 0),
        )
    }

    fn programs_folder(token: HANDLE) -> Result<PathBuf, String> {
        let mut raw = ptr::null_mut();
        let result = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, 0, token, &mut raw) };
        if result < 0 || raw.is_null() {
            if !raw.is_null() {
                unsafe { CoTaskMemFree(raw.cast()) };
            }
            return Err(format!("user_launch_programs_folder_failed:{result:#010x}"));
        }
        let mut len = 0;
        while len < 32768 && unsafe { *raw.add(len) } != 0 {
            len += 1;
        }
        let path = if len == 32768 {
            Err("user_launch_programs_folder_unbounded".to_string())
        } else {
            Ok(PathBuf::from(OsString::from_wide(unsafe {
                std::slice::from_raw_parts(raw, len)
            })))
        };
        unsafe { CoTaskMemFree(raw.cast()) };
        let path = path?;
        if !path.is_absolute() {
            return Err("user_launch_programs_folder_not_absolute".to_string());
        }
        Ok(path)
    }

    struct PinnedDirectory {
        path: PathBuf,
        handle: OwnedHandle,
    }

    fn pin_ancestry(root: &Path) -> Result<Vec<PinnedDirectory>, String> {
        let mut paths = root
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<Vec<_>>();
        paths.reverse();
        if paths.len() > 32 {
            return Err("user_launch_ancestry_unbounded".to_string());
        }
        paths
            .into_iter()
            .map(|path| {
                let raw = unsafe {
                    CreateFileW(
                        wide(path).as_ptr(),
                        FILE_READ_ATTRIBUTES,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                        ptr::null_mut(),
                    )
                };
                if invalid_handle(raw) {
                    return Err(last_error("user_launch_directory_open_failed"));
                }
                let pinned = PinnedDirectory {
                    path: path.to_path_buf(),
                    handle: OwnedHandle(raw),
                };
                pinned.validate()?;
                Ok(pinned)
            })
            .collect()
    }

    impl PinnedDirectory {
        fn validate(&self) -> Result<(), String> {
            let info = file_info(self.handle.0)?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || !same_path(&final_path(self.handle.0)?, &self.path)
            {
                return Err("user_launch_directory_identity_invalid".to_string());
            }
            Ok(())
        }
    }

    fn revalidate_destination(
        token: HANDLE,
        root: &Path,
        ancestry: &[PinnedDirectory],
    ) -> Result<(), String> {
        for directory in ancestry {
            directory.validate()?;
        }
        if !same_path(&programs_folder(token)?, root) {
            return Err("user_launch_programs_folder_changed".to_string());
        }
        Ok(())
    }

    struct NewEntry {
        file: File,
        committed: bool,
    }

    impl NewEntry {
        fn create(path: &Path) -> Result<Option<Self>, String> {
            let raw = unsafe {
                CreateFileW(
                    wide(path).as_ptr(),
                    0x4000_0000 | FILE_READ_ATTRIBUTES | DELETE,
                    0,
                    ptr::null(),
                    CREATE_NEW,
                    FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            };
            if invalid_handle(raw) {
                let error = unsafe { GetLastError() };
                return if is_creation_collision(error) {
                    Ok(None)
                } else {
                    Err(format!("user_launch_create_failed:{error}"))
                };
            }
            Ok(Some(Self {
                file: unsafe { File::from_raw_handle(raw) },
                committed: false,
            }))
        }

        fn rollback(&mut self) -> Result<(), String> {
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            if unsafe {
                SetFileInformationByHandle(
                    self.file.as_raw_handle(),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0
            {
                return Err(last_error("user_launch_rollback_failed"));
            }
            self.committed = true;
            Ok(())
        }

        fn validate(&self, path: &Path, size: u64) -> Result<(), String> {
            let handle = self.file.as_raw_handle();
            let info = file_info(handle)?;
            let actual_size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
            if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
            {
                return Err("user_launch_created_entry_attributes_invalid".to_string());
            }
            if info.nNumberOfLinks != 1 {
                return Err("user_launch_created_entry_link_count_invalid".to_string());
            }
            if actual_size != size {
                return Err("user_launch_created_entry_size_invalid".to_string());
            }
            if !same_path(&final_path(handle)?, path) {
                return Err("user_launch_created_entry_path_invalid".to_string());
            }
            Ok(())
        }
    }

    impl Drop for NewEntry {
        fn drop(&mut self) {
            if !self.committed {
                // Roll back only this CREATE_NEW handle. Never reopen or unlink a path.
                let _ = self.rollback();
            }
        }
    }

    fn file_info(handle: HANDLE) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
            Err(last_error("user_launch_file_info_failed"))
        } else {
            Ok(info)
        }
    }

    fn final_path(handle: HANDLE) -> Result<PathBuf, String> {
        let required = unsafe { GetFinalPathNameByHandleW(handle, ptr::null_mut(), 0, 0) };
        if required == 0 || required > 32768 {
            return Err(last_error("user_launch_final_path_failed"));
        }
        let mut buffer = vec![0u16; required as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0)
        };
        if written == 0 || written as usize >= buffer.len() {
            return Err(last_error("user_launch_final_path_failed"));
        }
        let units = &buffer[..written as usize];
        let unc_prefix = r"\\?\UNC\".encode_utf16().collect::<Vec<_>>();
        if units.len() >= unc_prefix.len()
            && units[..unc_prefix.len()]
                .iter()
                .zip(&unc_prefix)
                .all(|(a, b)| {
                    (*a <= 0x7f && *b <= 0x7f) && (*a as u8).eq_ignore_ascii_case(&(*b as u8))
                })
        {
            let mut unc = vec![b'\\' as u16, b'\\' as u16];
            unc.extend_from_slice(&units[unc_prefix.len()..]);
            return Ok(PathBuf::from(OsString::from_wide(&unc)));
        }
        Ok(
            crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(
                PathBuf::from(OsString::from_wide(units)),
            ),
        )
    }

    fn same_path(left: &Path, right: &Path) -> bool {
        left.as_os_str()
            .as_encoded_bytes()
            .eq_ignore_ascii_case(right.as_os_str().as_encoded_bytes())
    }
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    fn invalid_handle(handle: HANDLE) -> bool {
        handle.is_null() || handle as isize == -1
    }
    fn last_error(context: &str) -> String {
        format!("{context}:{}", unsafe { GetLastError() })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn exclusive_creation_preserves_foreign_and_identical_existing_bytes() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(ENTRY_NAME);
            for bytes in [
                b"foreign malformed shortcut".as_slice(),
                b"same bytes as our prior entry".as_slice(),
            ] {
                std::fs::write(&path, bytes).unwrap();
                let outcome = publish_missing_entry(
                    || NewEntry::create(&path),
                    |_| panic!("existing entry cannot be adopted"),
                )
                .unwrap();
                assert_eq!(outcome, UserLaunchOutcome::ExistingEntryPreserved);
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
        }

        #[test]
        fn only_newly_created_handle_is_rolled_back_on_failure() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(ENTRY_NAME);
            let outcome = publish_missing_entry(
                || NewEntry::create(&path),
                |mut entry| {
                    entry.file.write_all(b"incomplete").unwrap();
                    Err("write interrupted".to_string())
                },
            );
            assert_eq!(outcome, Err("write interrupted".to_string()));
            assert!(!path.exists());
            assert!(directory.path().is_dir());
        }

        #[test]
        fn pinned_creation_commits_only_complete_new_entry() {
            let directory = tempfile::tempdir().unwrap();
            // Temp directories may use a short-name alias. The production destination
            // is a known-folder path that must pass the canonical ancestry check.
            let root = crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(
                directory.path().canonicalize().unwrap(),
            );
            let _ancestry = pin_ancestry(&root).unwrap();
            let path = root.join(ENTRY_NAME);
            let bytes = b"complete shortcut bytes";
            let outcome = publish_missing_entry(
                || NewEntry::create(&path),
                |mut entry| {
                    entry.file.write_all(bytes).unwrap();
                    entry.file.sync_all().unwrap();
                    entry.validate(&path, bytes.len() as u64)?;
                    entry.committed = true;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(outcome, UserLaunchOutcome::Created);
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }

        #[test]
        fn pinned_creation_rejects_wrong_size_and_destination() {
            let directory = tempfile::tempdir().unwrap();
            let root = crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(
                directory.path().canonicalize().unwrap(),
            );
            let path = root.join(ENTRY_NAME);
            let mut entry = NewEntry::create(&path).unwrap().unwrap();
            entry.file.write_all(b"complete").unwrap();
            entry.file.sync_all().unwrap();
            assert_eq!(
                entry.validate(&path, 9),
                Err("user_launch_created_entry_size_invalid".into())
            );
            assert_eq!(
                entry.validate(&root.join("another entry.lnk"), 8),
                Err("user_launch_created_entry_path_invalid".into())
            );
            drop(entry);
            assert!(!path.exists());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn refused_authority_never_reaches_install_or_profile_resolution() {
        for (elevated, session, service, impersonating) in [
            (true, 1, false, false),  // elevated invoking account
            (false, 0, false, false), // noninteractive session
            (false, 1, true, false),  // service account even outside session zero
            (false, 1, false, true),  // thread acting for another token
            (true, 0, true, true),
        ] {
            let outcome = with_user_context(elevated, session, service, impersonating, || {
                panic!("must not resolve installation or another user's profile")
            })
            .unwrap();
            assert_eq!(outcome, UserLaunchOutcome::SkippedContext);
        }
        let created = with_user_context(false, 3, false, false, || Ok(UserLaunchOutcome::Created));
        assert_eq!(created, Ok(UserLaunchOutcome::Created));
        let unverified = with_user_context(false, 3, false, false, || {
            Err("installed image unverified".to_string())
        });
        assert_eq!(unverified, Err("installed image unverified".to_string()));
    }

    #[test]
    fn existing_entry_is_preserved_without_publishing_or_adopting_it() {
        let publish_calls = Cell::new(0);
        for _ in 0..2 {
            let outcome = publish_missing_entry(
                || Ok(None::<()>),
                |_| {
                    publish_calls.set(1);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(outcome, UserLaunchOutcome::ExistingEntryPreserved);
        }
        assert_eq!(publish_calls.get(), 0);
    }

    #[test]
    fn creation_failure_does_not_retry_and_publish_failure_is_not_success() {
        let create_calls = Cell::new(0);
        let failed = publish_missing_entry::<()>(
            || {
                create_calls.set(create_calls.get() + 1);
                Err("denied".into())
            },
            |_| panic!("must not publish"),
        );
        assert_eq!(failed, Err("denied".to_string()));
        assert_eq!(create_calls.get(), 1);
        assert_eq!(
            publish_missing_entry(|| Ok(Some(())), |_| Err("write failed".into())),
            Err("write failed".to_string())
        );
        assert_eq!(
            publish_missing_entry(|| Ok(Some(())), |_| Ok(())),
            Ok(UserLaunchOutcome::Created)
        );
    }

    #[test]
    fn only_explicit_already_existing_errors_mean_preserved_collision() {
        for error in [0, 2, 3, 5, 32, 79, 80, 81, 182, 183, 184] {
            assert_eq!(is_creation_collision(error), error == 80 || error == 183);
        }
    }
}
