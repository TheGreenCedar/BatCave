#![cfg(target_os = "windows")]

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_READ, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

const OWNED_IMAGE_NAME: &str = "batcave-owned-nsis-probe.exe";
const CHILD_TIMEOUT: Duration = Duration::from_millis(750);
const WORKER_SLEEP: Duration = Duration::from_secs(30);
static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
enum Scenario {
    Clean,
    Denied,
    Timeout,
    ChildFailure,
    Residue,
    ProcessTreeTimeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Disposition {
    SourceContractComplete,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FailureKind {
    DeniedBeforeChild,
    TimedOut,
    ChildFailed,
    Residue,
    OwnershipFailed,
    CleanupFailed,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct SourceEvidence {
    proof_scope: &'static str,
    profile: &'static str,
    owned_image_consumed: bool,
    process_tree_settled: bool,
    windows_service_etw_out_of_scope: bool,
    public_artifact_verified: bool,
    native_proven: bool,
    release_evidence: Option<serde_json::Value>,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct Outcome {
    disposition: Disposition,
    failure: Option<FailureKind>,
    child_started: bool,
    owned_image_consumed: bool,
    process_tree_settled: bool,
    private_root_removed: bool,
    source_evidence: Option<SourceEvidence>,
}

struct OwnedImage {
    root: PathBuf,
    root_handle: RootHandle,
    path: PathBuf,
    handle: File,
    size: u64,
    sha256: [u8; 32],
    identity: FileIdentity,
    cleanup_leaves: Vec<OwnedCleanupLeaf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume_serial: u32,
    file_index: u64,
}

struct OwnedCleanupLeaf {
    path: PathBuf,
    identity: FileIdentity,
}

impl OwnedImage {
    fn acquire() -> Result<Self, FailureKind> {
        let source = std::env::current_exe().map_err(|_| FailureKind::OwnershipFailed)?;
        let bytes = fs::read(source).map_err(|_| FailureKind::OwnershipFailed)?;
        let root = std::env::temp_dir().join(format!(
            "batcave-windows-nsis-{}-{}",
            std::process::id(),
            ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).map_err(|_| FailureKind::OwnershipFailed)?;
        let root_handle = RootHandle::open(&root)?;
        let path = root.join(OWNED_IMAGE_NAME);
        let mut writer = OpenOptions::new()
            .create_new(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|_| FailureKind::OwnershipFailed)?;
        writer
            .write_all(&bytes)
            .and_then(|_| writer.sync_all())
            .map_err(|_| FailureKind::OwnershipFailed)?;
        drop(writer);

        let mut handle = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|_| FailureKind::OwnershipFailed)?;
        let observed = digest_handle(&mut handle).map_err(|_| FailureKind::OwnershipFailed)?;
        let sha256 = Sha256::digest(&bytes).into();
        if observed != sha256 {
            return Err(FailureKind::OwnershipFailed);
        }
        let information = file_information(&handle).map_err(|_| FailureKind::OwnershipFailed)?;
        if information.number_of_links != 1 {
            return Err(FailureKind::OwnershipFailed);
        }
        Ok(Self {
            root,
            root_handle,
            path,
            handle,
            size: bytes.len() as u64,
            sha256,
            identity: information.identity,
            cleanup_leaves: Vec::new(),
        })
    }

    fn revalidate(&mut self) -> Result<(), FailureKind> {
        let metadata = self
            .handle
            .metadata()
            .map_err(|_| FailureKind::OwnershipFailed)?;
        if !metadata.is_file()
            || metadata.len() != self.size
            || digest_handle(&mut self.handle).map_err(|_| FailureKind::OwnershipFailed)?
                != self.sha256
        {
            return Err(FailureKind::OwnershipFailed);
        }
        Ok(())
    }

    fn add_owned_residue(&mut self) -> Result<(), FailureKind> {
        let path = self.root.join("unexpected.residue");
        let mut writer = OpenOptions::new()
            .create_new(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|_| FailureKind::OwnershipFailed)?;
        writer
            .write_all(b"owned hostile residue")
            .and_then(|_| writer.sync_all())
            .map_err(|_| FailureKind::OwnershipFailed)?;
        drop(writer);
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|_| FailureKind::OwnershipFailed)?;
        let information = file_information(&handle).map_err(|_| FailureKind::OwnershipFailed)?;
        if information.number_of_links != 1 {
            return Err(FailureKind::OwnershipFailed);
        }
        self.cleanup_leaves.push(OwnedCleanupLeaf {
            path,
            identity: information.identity,
        });
        Ok(())
    }

    fn cleanup(self) -> Result<(), FailureKind> {
        self.cleanup_with_hook(|_| {})
    }

    fn cleanup_with_hook<F>(mut self, before_root_delete: F) -> Result<(), FailureKind>
    where
        F: FnOnce(&Path),
    {
        self.revalidate()?;
        let OwnedImage {
            root,
            root_handle,
            path,
            handle,
            size,
            sha256,
            identity,
            cleanup_leaves,
        } = self;
        drop(handle);

        delete_exact_leaf(&path, identity, Some((size, sha256)))?;
        for leaf in cleanup_leaves {
            delete_exact_leaf(&leaf.path, leaf.identity, None)?;
        }

        before_root_delete(&root);
        root_handle.delete()?;
        if root.exists() {
            return Err(FailureKind::CleanupFailed);
        }
        Ok(())
    }
}

struct RootHandle(HANDLE);

impl RootHandle {
    fn open(root: &Path) -> Result<Self, FailureKind> {
        let path = wide(root.as_os_str());
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES | DELETE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(FailureKind::OwnershipFailed);
        }
        Ok(Self(handle))
    }

    fn delete(self) -> Result<(), FailureKind> {
        mark_delete(self.0)?;
        drop(self);
        Ok(())
    }
}

impl Drop for RootHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn digest_handle(file: &mut File) -> std::io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(hash.finalize().into())
}

struct FileInformation {
    identity: FileIdentity,
    number_of_links: u32,
}

fn file_information(file: &File) -> std::io::Result<FileInformation> {
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FileInformation {
        identity: FileIdentity {
            volume_serial: information.dwVolumeSerialNumber,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        },
        number_of_links: information.nNumberOfLinks,
    })
}

fn delete_exact_leaf(
    path: &Path,
    expected_identity: FileIdentity,
    expected_content: Option<(u64, [u8; 32])>,
) -> Result<(), FailureKind> {
    let mut handle = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | DELETE)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .map_err(|_| FailureKind::CleanupFailed)?;
    if file_information(&handle)
        .map_err(|_| FailureKind::CleanupFailed)?
        .identity
        != expected_identity
    {
        return Err(FailureKind::CleanupFailed);
    }
    if let Some((size, sha256)) = expected_content {
        let metadata = handle.metadata().map_err(|_| FailureKind::CleanupFailed)?;
        if metadata.len() != size
            || digest_handle(&mut handle).map_err(|_| FailureKind::CleanupFailed)? != sha256
        {
            return Err(FailureKind::CleanupFailed);
        }
    }
    mark_delete(handle.as_raw_handle() as HANDLE)?;
    // Windows removes the pending name from the link count immediately. Zero
    // therefore proves this was the sole link while delete-pending blocks new ones.
    let delete_pending_information =
        file_information(&handle).map_err(|_| FailureKind::CleanupFailed)?;
    if delete_pending_information.identity != expected_identity
        || delete_pending_information.number_of_links != 0
    {
        return Err(FailureKind::CleanupFailed);
    }
    drop(handle);
    Ok(())
}

fn mark_delete(handle: HANDLE) -> Result<(), FailureKind> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let ok = unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            &disposition as *const _ as _,
            std::mem::size_of_val(&disposition) as u32,
        )
    };
    (ok != 0).then_some(()).ok_or(FailureKind::CleanupFailed)
}

struct Job(HANDLE);

impl Job {
    fn new() -> Result<Self, FailureKind> {
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(FailureKind::OwnershipFailed);
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            unsafe { CloseHandle(handle) };
            return Err(FailureKind::OwnershipFailed);
        }
        Ok(Self(handle))
    }

    fn active_processes(&self) -> Result<u32, FailureKind> {
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as _,
                std::mem::size_of_val(&accounting) as u32,
                null_mut(),
            )
        };
        (ok != 0)
            .then_some(accounting.ActiveProcesses)
            .ok_or(FailureKind::OwnershipFailed)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        loop {
            unsafe { TerminateJobObject(self.0, 1) };
            if matches!(self.active_processes(), Ok(0)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe { CloseHandle(self.0) };
    }
}

struct Child {
    process: HANDLE,
    thread: HANDLE,
}

impl Drop for Child {
    fn drop(&mut self) {
        while unsafe { WaitForSingleObject(self.process, 0) } != WAIT_OBJECT_0 {
            unsafe { TerminateProcess(self.process, 1) };
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

fn fixed_worker(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::Clean | Scenario::Residue => "worker_clean",
        Scenario::ChildFailure => "worker_failure",
        Scenario::Timeout => "worker_slow",
        Scenario::ProcessTreeTimeout => "worker_process_tree",
        Scenario::Denied => unreachable!("denial occurs before child creation"),
    }
}

fn spawn_suspended(path: &Path, worker: &str) -> Result<Child, FailureKind> {
    let application = wide(path.as_os_str());
    let mut command = wide(std::ffi::OsStr::new(&format!(
        "\"{}\" --exact {worker} --nocapture",
        path.display()
    )));
    let environment = [0u16, 0u16];
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            null(),
            &startup,
            &mut information,
        )
    };
    if created == 0 {
        return Err(
            if std::io::Error::last_os_error().raw_os_error() == Some(ERROR_CANCELLED as i32) {
                FailureKind::DeniedBeforeChild
            } else {
                FailureKind::OwnershipFailed
            },
        );
    }
    Ok(Child {
        process: information.hProcess,
        thread: information.hThread,
    })
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}

fn execute_fixed_child(
    image: &mut OwnedImage,
    scenario: Scenario,
) -> Result<(bool, bool), FailureKind> {
    image.revalidate()?;
    let job = Job::new()?;
    let child = spawn_suspended(&image.path, fixed_worker(scenario))?;
    if unsafe { AssignProcessToJobObject(job.0, child.process) } == 0 {
        if settle_after_error(&job, &child, false).is_err() {
            settle_fail_safe(&job, &child);
        }
        return Err(FailureKind::OwnershipFailed);
    }
    if unsafe { ResumeThread(child.thread) } == u32::MAX {
        if settle_after_error(&job, &child, false).is_err() {
            settle_fail_safe(&job, &child);
        }
        return Err(FailureKind::OwnershipFailed);
    }

    let wait = unsafe { WaitForSingleObject(child.process, CHILD_TIMEOUT.as_millis() as u32) };
    if wait == WAIT_TIMEOUT {
        let expected_processes = if matches!(scenario, Scenario::ProcessTreeTimeout) {
            2
        } else {
            1
        };
        let active = match job.active_processes() {
            Ok(active) => active,
            Err(_) => {
                settle_fail_safe(&job, &child);
                return Err(FailureKind::OwnershipFailed);
            }
        };
        if active < expected_processes {
            settle_fail_safe(&job, &child);
            return Err(FailureKind::OwnershipFailed);
        }
        if unsafe { TerminateJobObject(job.0, 124) } == 0
            || unsafe { WaitForSingleObject(child.process, 5_000) } != WAIT_OBJECT_0
        {
            return Err(FailureKind::OwnershipFailed);
        }
        for _ in 0..100 {
            match job.active_processes() {
                Ok(0) => {
                    image.revalidate()?;
                    return Ok((true, true));
                }
                Ok(_) => {}
                Err(_) => {
                    settle_fail_safe(&job, &child);
                    return Err(FailureKind::OwnershipFailed);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        return Err(FailureKind::OwnershipFailed);
    }
    if wait != WAIT_OBJECT_0 {
        settle_fail_safe(&job, &child);
        return Err(FailureKind::OwnershipFailed);
    }
    let mut exit_code = 0;
    if unsafe { GetExitCodeProcess(child.process, &mut exit_code) } == 0 {
        settle_fail_safe(&job, &child);
        return Err(FailureKind::OwnershipFailed);
    }
    for _ in 0..100 {
        match job.active_processes() {
            Ok(0) => {
                image.revalidate()?;
                return Ok((false, exit_code == 0));
            }
            Ok(_) => {}
            Err(_) => {
                settle_fail_safe(&job, &child);
                return Err(FailureKind::OwnershipFailed);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    settle_fail_safe(&job, &child);
    Err(FailureKind::OwnershipFailed)
}

fn settle_after_error(
    job: &Job,
    child: &Child,
    force_primary_failure: bool,
) -> Result<(), FailureKind> {
    if force_primary_failure
        || unsafe { TerminateJobObject(job.0, 1) } == 0
        || unsafe { TerminateProcess(child.process, 1) } == 0
        || unsafe { WaitForSingleObject(child.process, 5_000) } != WAIT_OBJECT_0
    {
        return Err(FailureKind::OwnershipFailed);
    }
    for _ in 0..100 {
        match job.active_processes() {
            Ok(0) => return Ok(()),
            Ok(_) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return Err(FailureKind::OwnershipFailed),
        }
    }
    Err(FailureKind::OwnershipFailed)
}

fn settle_fail_safe(job: &Job, child: &Child) {
    loop {
        unsafe {
            TerminateJobObject(job.0, 1);
            TerminateProcess(child.process, 1);
        }
        let child_settled = unsafe { WaitForSingleObject(child.process, 100) } == WAIT_OBJECT_0;
        if child_settled && matches!(job.active_processes(), Ok(0)) {
            return;
        }
    }
}

fn run(scenario: Scenario) -> Outcome {
    let mut image = match OwnedImage::acquire() {
        Ok(image) => image,
        Err(failure) => return failed(failure, false, false, false),
    };

    let (failure, child_started, consumed, settled) = if matches!(scenario, Scenario::Denied) {
        (Some(FailureKind::DeniedBeforeChild), false, false, true)
    } else {
        match execute_fixed_child(&mut image, scenario) {
            Ok((timed_out, _)) if timed_out => (Some(FailureKind::TimedOut), true, true, true),
            Ok((_, false)) => (Some(FailureKind::ChildFailed), true, true, true),
            Ok((_, true)) if matches!(scenario, Scenario::Residue) => {
                match image.add_owned_residue() {
                    Ok(()) => (Some(FailureKind::Residue), true, true, true),
                    Err(failure) => (Some(failure), true, false, false),
                }
            }
            Ok((_, true)) => (None, true, true, true),
            Err(failure) => (Some(failure), true, false, false),
        }
    };

    let cleanup = image.cleanup();
    if let Err(failure) = cleanup {
        return failed(failure, child_started, consumed, settled);
    }
    let source_evidence = failure.is_none().then_some(SourceEvidence {
        proof_scope: "windows_nsis_owned_adapter_source_contract",
        profile: "windows:nsis",
        owned_image_consumed: true,
        process_tree_settled: true,
        windows_service_etw_out_of_scope: true,
        public_artifact_verified: false,
        native_proven: false,
        release_evidence: None,
    });
    Outcome {
        disposition: if failure.is_none() {
            Disposition::SourceContractComplete
        } else {
            Disposition::Failed
        },
        failure,
        child_started,
        owned_image_consumed: consumed,
        process_tree_settled: settled,
        private_root_removed: true,
        source_evidence,
    }
}

fn failed(failure: FailureKind, child_started: bool, consumed: bool, settled: bool) -> Outcome {
    Outcome {
        disposition: Disposition::Failed,
        failure: Some(failure),
        child_started,
        owned_image_consumed: consumed,
        process_tree_settled: settled,
        private_root_removed: false,
        source_evidence: None,
    }
}

fn running_from_owned_image() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name == OWNED_IMAGE_NAME))
        .unwrap_or(false)
}

#[test]
fn worker_clean() {
    if running_from_owned_image() {}
}

#[test]
fn worker_failure() {
    if running_from_owned_image() {
        panic!("fixed worker failure");
    }
}

#[test]
fn worker_slow() {
    if running_from_owned_image() {
        std::thread::sleep(WORKER_SLEEP);
    }
}

#[test]
fn worker_process_tree() {
    if running_from_owned_image() {
        let executable = std::env::current_exe().expect("owned worker image");
        let mut child = Command::new(executable)
            .env_clear()
            .args(["--exact", "worker_grandchild", "--nocapture"])
            .spawn()
            .expect("fixed grandchild starts");
        let _ = child.wait();
    }
}

#[test]
fn worker_grandchild() {
    if running_from_owned_image() {
        std::thread::sleep(WORKER_SLEEP);
    }
}

#[test]
fn exact_owned_image_runs_and_evidence_is_derived_after_settlement() {
    let outcome = run(Scenario::Clean);
    assert_eq!(outcome.disposition, Disposition::SourceContractComplete);
    assert!(outcome.child_started);
    assert!(outcome.owned_image_consumed);
    assert!(outcome.process_tree_settled);
    assert!(outcome.private_root_removed);
    let evidence = outcome.source_evidence.expect("settled source evidence");
    assert!(evidence.windows_service_etw_out_of_scope);
    assert!(!evidence.public_artifact_verified);
    assert!(!evidence.native_proven);
    assert!(evidence.release_evidence.is_none());
}

#[test]
fn denial_timeout_failure_and_residue_remain_distinct_and_emit_no_evidence() {
    for (scenario, expected, started, consumed) in [
        (
            Scenario::Denied,
            FailureKind::DeniedBeforeChild,
            false,
            false,
        ),
        (Scenario::Timeout, FailureKind::TimedOut, true, true),
        (Scenario::ChildFailure, FailureKind::ChildFailed, true, true),
        (Scenario::Residue, FailureKind::Residue, true, true),
    ] {
        let outcome = run(scenario);
        assert_eq!(outcome.disposition, Disposition::Failed);
        assert_eq!(outcome.failure, Some(expected));
        assert_eq!(outcome.child_started, started);
        assert_eq!(outcome.owned_image_consumed, consumed);
        assert!(outcome.process_tree_settled);
        assert!(outcome.private_root_removed);
        assert!(outcome.source_evidence.is_none());
    }
}

#[test]
fn timeout_settles_the_entire_owned_process_tree() {
    let outcome = run(Scenario::ProcessTreeTimeout);
    assert_eq!(outcome.failure, Some(FailureKind::TimedOut));
    assert!(outcome.owned_image_consumed);
    assert!(outcome.process_tree_settled);
    assert!(outcome.private_root_removed);
    assert!(outcome.source_evidence.is_none());
}

#[test]
fn owned_handle_blocks_replacement_and_deletion_until_settlement() {
    let image = OwnedImage::acquire().expect("owned image");
    assert!(OpenOptions::new().write(true).open(&image.path).is_err());
    assert!(fs::remove_file(&image.path).is_err());
    assert!(fs::rename(&image.root, image.root.with_extension("moved")).is_err());
    image.cleanup().expect("owned cleanup");
}

#[test]
fn handle_authorized_cleanup_blocks_root_swap_at_the_deletion_seam() {
    let image = OwnedImage::acquire().expect("owned image");
    let root = image.root.clone();
    let moved = root.with_extension("moved");
    image
        .cleanup_with_hook(|owned_root| {
            assert!(fs::rename(owned_root, &moved).is_err());
            assert!(fs::create_dir(owned_root).is_err());
        })
        .expect("handle-authorized cleanup");
    assert!(!root.exists());
    assert!(!moved.exists());
}

#[test]
fn delete_pending_cleanup_rejects_an_external_hard_link_without_emitting_evidence() {
    let image = OwnedImage::acquire().expect("owned image");
    let root = image.root.clone();
    let external_link = std::env::temp_dir().join(format!(
        "batcave-windows-nsis-hardlink-{}-{}",
        std::process::id(),
        ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::hard_link(&image.path, &external_link).expect("external hard link");

    assert_eq!(image.cleanup(), Err(FailureKind::CleanupFailed));
    assert!(external_link.exists());
    assert!(root.exists());

    fs::remove_file(&external_link).expect("remove known hostile link");
    fs::remove_dir(&root).expect("remove now-empty owned root");
}

#[test]
fn unassigned_suspended_child_and_primary_termination_failure_retain_handles_until_settlement() {
    let mut image = OwnedImage::acquire().expect("owned image");
    let job = Job::new().expect("owned job");
    let child = spawn_suspended(&image.path, "worker_slow").expect("suspended owned child");

    assert_eq!(
        settle_after_error(&job, &child, true),
        Err(FailureKind::OwnershipFailed)
    );
    assert_ne!(
        unsafe { WaitForSingleObject(child.process, 0) },
        WAIT_OBJECT_0
    );

    settle_fail_safe(&job, &child);
    assert_eq!(
        unsafe { WaitForSingleObject(child.process, 0) },
        WAIT_OBJECT_0
    );
    assert_eq!(job.active_processes(), Ok(0));
    image.revalidate().expect("owned bytes retained");
    drop(child);
    drop(job);
    image.cleanup().expect("owned cleanup");
}

#[test]
fn entry_has_no_caller_command_path_argument_environment_or_evidence_input() {
    let entry: fn(Scenario) -> Outcome = run;
    let rendered = serde_json::to_string(&entry(Scenario::Clean)).expect("serialize outcome");
    assert!(!rendered.contains("C:\\"));
    assert!(!rendered.contains("command"));
    assert!(!rendered.contains("environment"));
    assert!(!rendered.contains("native_proven\":true"));
    assert!(!rendered.contains("release_evidence\":{"));
}
