#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use crate::contracts::{
    AccessState, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
    ProcessMetricQuality, ProcessSample,
};
use crate::{
    network_attribution::ProcessGeneration,
    workload_identity::{verified_parent_indices, BatCaveWorkloadContext},
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkloadProcessIdentity {
    generation: ProcessGeneration,
    sample_exe: String,
    image_path: String,
    file_identity: (u32, u64),
    principal_identity: [u8; 32],
    session_id: u32,
}

fn unique_generation_index(
    processes: &[ProcessSample],
    generation: ProcessGeneration,
) -> Option<usize> {
    if generation.pid == 0 || generation.start_time_ms == 0 {
        return None;
    }
    let mut matches = processes
        .iter()
        .enumerate()
        .filter(|(_, row)| row.pid.parse::<u32>().ok() == Some(generation.pid));
    let (index, row) = matches.next()?;
    (matches.next().is_none()
        && ProcessGeneration::from_process(row) == Some(generation)
        && !row.exe.is_empty())
    .then_some(index)
}

fn canonical_windows_path_key(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn approved_webview_image(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| {
        let Some(relative) = path.strip_prefix(&format!("{root}\\")) else {
            return false;
        };
        let Some((version, leaf)) = relative.split_once('\\') else {
            return false;
        };
        leaf == "msedgewebview2.exe"
            && version.split('.').count() == 4
            && version
                .split('.')
                .all(|part| !part.is_empty() && part.parse::<u32>().is_ok())
    })
}

fn context_from_workload_identities(
    processes: &[ProcessSample],
    desktop: ProcessGeneration,
    service: Option<ProcessGeneration>,
    identities: &HashMap<usize, WorkloadProcessIdentity>,
    webview_roots: &[String],
) -> Option<BatCaveWorkloadContext> {
    let desktop_index = unique_generation_index(processes, desktop)?;
    let mut pid_counts = HashMap::<u32, usize>::new();
    for row in processes {
        if let Ok(pid) = row.pid.parse::<u32>() {
            *pid_counts.entry(pid).or_default() += 1;
        }
    }
    let belongs_to_row = |index: usize, identity: &WorkloadProcessIdentity| {
        identity.generation.pid > 0
            && pid_counts.get(&identity.generation.pid) == Some(&1)
            && ProcessGeneration::from_process(&processes[index]) == Some(identity.generation)
            && identity.sample_exe == processes[index].exe
            && !identity.image_path.is_empty()
            && identity.file_identity.1 != 0
            && identity.principal_identity != [0; 32]
    };
    let desktop_identity = identities.get(&desktop_index)?;
    if !belongs_to_row(desktop_index, desktop_identity) {
        return None;
    }
    let parents = verified_parent_indices(processes);
    let mut approved = HashSet::from([desktop_index]);
    let mut children = vec![Vec::new(); processes.len()];
    for (index, parent) in parents.iter().enumerate() {
        if let Some(parent) = parent {
            children[*parent].push(index);
        }
    }
    let mut pending = vec![desktop_index];
    while let Some(parent) = pending.pop() {
        for &index in &children[parent] {
            let Some(identity) = identities.get(&index) else {
                continue;
            };
            if belongs_to_row(index, identity)
                && identity.principal_identity == desktop_identity.principal_identity
                && identity.session_id == desktop_identity.session_id
                && ((identity.image_path == desktop_identity.image_path
                    && identity.file_identity == desktop_identity.file_identity)
                    || approved_webview_image(&identity.image_path, webview_roots))
            {
                approved.insert(index);
                pending.push(index);
            }
        }
    }
    if let Some(index) =
        service.and_then(|generation| unique_generation_index(processes, generation))
    {
        if identities
            .get(&index)
            .is_some_and(|identity| belongs_to_row(index, identity))
        {
            approved.insert(index);
        }
    }
    let members = processes
        .iter()
        .enumerate()
        .filter(|(index, _)| approved.contains(index))
        .map(|(index, _)| identities[&index].generation)
        .collect();
    Some(BatCaveWorkloadContext { desktop, members })
}

/// Only fresh sample rows and the current transport-authenticated service generation enter here.
#[cfg(windows)]
pub(crate) fn batcave_workload_context(
    processes: &[ProcessSample],
    verified_service: Option<ProcessGeneration>,
) -> Option<BatCaveWorkloadContext> {
    workload_probe::observe(processes, verified_service)
}

#[cfg(windows)]
mod workload_probe {
    use super::*;
    use crate::collector_service::windows_transport::token_evidence;
    use std::{
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr,
    };
    use windows_sys::{
        core::GUID,
        Win32::{
            Foundation::{FILETIME, STILL_ACTIVE},
            Security::TOKEN_QUERY,
            Storage::FileSystem::{
                CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
                FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_READ_DATA,
                FILE_SHARE_READ, OPEN_EXISTING,
            },
            System::{
                Com::CoTaskMemFree,
                Threading::{GetExitCodeProcess, OpenProcessToken},
            },
            UI::Shell::{
                FOLDERID_LocalAppData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86,
                SHGetKnownFolderPath,
            },
        },
    };

    struct WorkloadHandle(HANDLE);
    impl WorkloadHandle {
        fn new(raw: HANDLE) -> Option<Self> {
            (!raw.is_null() && raw != INVALID_HANDLE_VALUE).then_some(Self(raw))
        }
    }
    impl Drop for WorkloadHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct NativeWorkloadProcess {
        identity: WorkloadProcessIdentity,
        process: ProcessHandle,
        image: WorkloadHandle,
    }

    fn canonical_image(process: HANDLE) -> Option<PathBuf> {
        PathBuf::from(query_process_image(process)?)
            .canonicalize()
            .ok()
    }
    fn path_key(path: &Path) -> String {
        canonical_windows_path_key(&path.to_string_lossy())
    }
    fn open_image(path: &Path) -> Option<WorkloadHandle> {
        let mut path = path.as_os_str().encode_wide().collect::<Vec<_>>();
        path.push(0);
        WorkloadHandle::new(unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_DATA | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        })
    }
    fn file_identity(image: HANDLE) -> Option<(u32, u64)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(image, &mut info) } == 0
            || info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
        {
            return None;
        }
        Some((
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
    fn principal(process: HANDLE) -> Option<([u8; 32], u32)> {
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let token = WorkloadHandle::new(token)?;
        let evidence = token_evidence(token.0).ok()?;
        Some((evidence.principal_identity, evidence.session_id))
    }
    fn active(process: HANDLE) -> bool {
        let mut code = 0;
        // Also require zero exit time: an exited process can itself return STILL_ACTIVE.
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetExitCodeProcess(process, &mut code) != 0
                && code == STILL_ACTIVE as u32
                && GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0
                && exited.dwHighDateTime == 0
                && exited.dwLowDateTime == 0
        }
    }
    fn probe(row: &ProcessSample) -> Option<NativeWorkloadProcess> {
        let generation = ProcessGeneration::from_process(row)?;
        if generation.pid == 0 || row.exe.is_empty() {
            return None;
        }
        let process = ProcessHandle::open(generation.pid)?;
        if query_process_start_time_ms(process.raw()) != Some(generation.start_time_ms)
            || !active(process.raw())
        {
            return None;
        }
        let path = canonical_image(process.raw())?;
        if Path::new(&row.exe).canonicalize().ok().as_ref() != Some(&path) {
            return None;
        }
        let image = open_image(&path)?;
        let file_identity = file_identity(image.0)?;
        let (principal_identity, session_id) = principal(process.raw())?;
        Some(NativeWorkloadProcess {
            identity: WorkloadProcessIdentity {
                generation,
                sample_exe: row.exe.clone(),
                image_path: path_key(&path),
                file_identity,
                principal_identity,
                session_id,
            },
            process,
            image,
        })
    }
    impl NativeWorkloadProcess {
        fn stable(&self) -> bool {
            let identity = &self.identity;
            if !active(self.process.raw())
                || query_process_start_time_ms(self.process.raw())
                    != Some(identity.generation.start_time_ms)
                || principal(self.process.raw())
                    != Some((identity.principal_identity, identity.session_id))
                || file_identity(self.image.0) != Some(identity.file_identity)
            {
                return false;
            }
            let Some(path) = canonical_image(self.process.raw()) else {
                return false;
            };
            path_key(&path) == identity.image_path
                && open_image(&path).and_then(|image| file_identity(image.0))
                    == Some(identity.file_identity)
        }
    }
    fn known_folder(folder: &GUID) -> Option<PathBuf> {
        let mut path = ptr::null_mut();
        if unsafe { SHGetKnownFolderPath(folder, 0, ptr::null_mut(), &mut path) } < 0 {
            if !path.is_null() {
                unsafe {
                    CoTaskMemFree(path.cast());
                }
            }
            return None;
        }
        if path.is_null() {
            return None;
        }
        let mut length = 0;
        while length < 32_768 && unsafe { *path.add(length) } != 0 {
            length += 1;
        }
        let result = (length < 32_768).then(|| {
            PathBuf::from(std::ffi::OsString::from_wide(unsafe {
                std::slice::from_raw_parts(path, length)
            }))
        });
        unsafe {
            CoTaskMemFree(path.cast());
        }
        result?.canonicalize().ok()
    }
    pub(super) fn observe(
        processes: &[ProcessSample],
        service: Option<ProcessGeneration>,
    ) -> Option<BatCaveWorkloadContext> {
        let row = processes
            .iter()
            .find(|row| row.pid.parse::<u32>().ok() == Some(std::process::id()))?;
        let desktop = ProcessGeneration::from_process(row)?;
        let desktop_index = unique_generation_index(processes, desktop)?;
        let desktop_path = std::env::current_exe().ok()?.canonicalize().ok()?;
        let parents = verified_parent_indices(processes);
        let mut children = vec![Vec::new(); processes.len()];
        for (index, parent) in parents.iter().enumerate() {
            if let Some(parent) = parent {
                children[*parent].push(index);
            }
        }
        let mut candidates = HashSet::from([desktop_index]);
        let mut pending = vec![desktop_index];
        while let Some(parent) = pending.pop() {
            for &child in &children[parent] {
                if candidates.insert(child) {
                    pending.push(child);
                }
            }
        }
        if let Some(index) =
            service.and_then(|generation| unique_generation_index(processes, generation))
        {
            candidates.insert(index);
        }
        // Handles survive all probes and rechecks, then only stable identities reach assembly.
        let pins = candidates
            .into_iter()
            .filter_map(|index| probe(&processes[index]).map(|pin| (index, pin)))
            .collect::<Vec<_>>();
        let identities = pins
            .iter()
            .filter(|(_, pin)| pin.stable())
            .map(|(index, pin)| (*index, pin.identity.clone()))
            .collect::<HashMap<_, _>>();
        if identities.get(&desktop_index)?.image_path != path_key(&desktop_path) {
            return None;
        }
        let roots = [
            &FOLDERID_ProgramFilesX86,
            &FOLDERID_ProgramFiles,
            &FOLDERID_LocalAppData,
        ]
        .into_iter()
        .filter_map(known_folder)
        .map(|folder| path_key(&folder.join("Microsoft\\EdgeWebView\\Application")))
        .collect::<Vec<_>>();
        context_from_workload_identities(processes, desktop, service, &identities, &roots)
    }
}

const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
const FILETIME_100NS_PER_MS: u64 = 10_000;
const PROCESS_PROBE_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, Default)]
struct ProcessProbeResults {
    memory: bool,
    io: bool,
    threads: bool,
    handles: bool,
}

#[cfg(windows)]
use std::mem::size_of;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        Threading::{
            GetProcessHandleCount, GetProcessIoCounters, GetProcessTimes, OpenProcess,
            QueryFullProcessImageNameW, IO_COUNTERS, PROCESS_QUERY_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
        },
    },
};

#[cfg(windows)]
pub fn collect_processes(_seq: u64) -> Result<Vec<ProcessSample>, String> {
    let snapshot_started_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "process_snapshot_clock_unavailable".to_string())?
        .as_millis() as u64;
    let snapshot = SnapshotHandle::create()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } == 0 {
        let error = unsafe { GetLastError() };
        if error == ERROR_NO_MORE_FILES {
            return Ok(Vec::new());
        }
        return Err(format!("process_snapshot_first_failed:{error}"));
    }

    let mut processes = Vec::new();
    loop {
        if let Some(sample) = sample_from_entry(&entry, snapshot_started_ms) {
            processes.push(sample);
        }

        if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(format!("process_snapshot_next_failed:{error}"));
        }
    }

    Ok(processes)
}

#[cfg(not(windows))]
pub fn collect_processes(_seq: u64) -> Result<Vec<ProcessSample>, String> {
    Err("windows_process_collector_requires_windows".to_string())
}

#[cfg(windows)]
fn sample_from_entry(entry: &PROCESSENTRY32W, snapshot_started_ms: u64) -> Option<ProcessSample> {
    let pid = entry.th32ProcessID;
    let parent_pid =
        (entry.th32ParentProcessID != 0).then(|| entry.th32ParentProcessID.to_string());
    let name = wide_null_terminated_to_string(&entry.szExeFile);

    let mut sample = ProcessSample {
        pid: pid.to_string(),
        parent_pid,
        start_time_ms: 0,
        name,
        exe: String::new(),
        status: "unknown".to_string(),
        cpu_percent: 0.0,
        kernel_cpu_percent: None,
        memory_bytes: 0,
        private_bytes: 0,
        virtual_memory_bytes: None,
        io_read_total_bytes: 0,
        io_write_total_bytes: 0,
        other_io_total_bytes: None,
        io_read_bps: 0,
        io_write_bps: 0,
        other_io_bps: None,
        network_received_bps: None,
        network_transmitted_bps: None,
        threads: entry.cntThreads,
        handles: 0,
        access_state: AccessState::Denied,
        quality: Some(process_metric_quality_from_probes(ProcessProbeResults {
            threads: true,
            ..ProcessProbeResults::default()
        })),
    };

    let Some(process) = ProcessHandle::open(pid) else {
        sample.access_state = resolve_access_state(0, PROCESS_PROBE_COUNT);
        return Some(sample);
    };
    // An entry predates OpenProcess. A later PID generation cannot inherit its parent/threads.
    let Some(start_time_ms) = query_process_start_time_ms(process.raw()) else {
        return Some(sample);
    };
    if !generation_precedes_snapshot(start_time_ms, snapshot_started_ms) {
        return None;
    }
    sample.start_time_ms = start_time_ms;

    let mut succeeded = 1;
    let mut failed = 0;
    let mut probes = ProcessProbeResults {
        threads: true,
        ..ProcessProbeResults::default()
    };

    match query_process_image(process.raw()) {
        Some(exe) => {
            sample.exe = exe;
            succeeded += 1;
        }
        None => failed += 1,
    }

    match query_process_memory(process.raw()) {
        Some(memory) => {
            sample.memory_bytes = memory.working_set_bytes;
            sample.private_bytes = memory.private_bytes;
            succeeded += 1;
            probes.memory = true;
        }
        None => failed += 1,
    }

    match query_process_io(process.raw()) {
        Some(io) => {
            sample.io_read_total_bytes = io.read_bytes;
            sample.io_write_total_bytes = io.write_bytes;
            sample.other_io_total_bytes = Some(io.other_bytes);
            succeeded += 1;
            probes.io = true;
        }
        None => failed += 1,
    }

    match query_process_handle_count(process.raw()) {
        Some(handles) => {
            sample.handles = handles;
            succeeded += 1;
            probes.handles = true;
        }
        None => failed += 1,
    }

    sample.access_state = resolve_access_state(succeeded, failed);
    sample.quality = Some(process_metric_quality_from_probes(probes));
    // All auxiliary reads use this retained HANDLE, then its generation is checked again.
    (query_process_start_time_ms(process.raw()) == Some(start_time_ms)).then_some(sample)
}

fn generation_precedes_snapshot(start_time_ms: u64, snapshot_started_ms: u64) -> bool {
    start_time_ms > 0 && start_time_ms < snapshot_started_ms
}

fn process_metric_quality_from_probes(probes: ProcessProbeResults) -> ProcessMetricQuality {
    let direct = |available: bool, unavailable_message: &str| {
        if available {
            MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi)
        } else {
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
                .with_limitation(MetricLimitationCode::AccessDenied, unavailable_message)
        }
    };

    ProcessMetricQuality {
        cpu: None,
        memory: Some(direct(
            probes.memory,
            "Native process memory counters are unavailable.",
        )),
        io: Some(if probes.io {
            MetricQualityInfo::new(MetricQuality::Native, MetricSource::DirectApi)
        } else {
            direct(
                false,
                "Native process read/write I/O counters are unavailable.",
            )
        }),
        other_io: Some(direct(
            probes.io,
            "Native process Other I/O counters are unavailable.",
        )),
        network: None,
        threads: Some(direct(
            probes.threads,
            "Toolhelp process thread count is unavailable.",
        )),
        handles: Some(direct(
            probes.handles,
            "Native process handle count is unavailable.",
        )),
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct SnapshotHandle {
    raw: HANDLE,
}

#[cfg(windows)]
impl SnapshotHandle {
    fn create() -> Result<Self, String> {
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if raw == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            Err(format!("create_process_snapshot_failed:{error}"))
        } else {
            Ok(Self { raw })
        }
    }

    fn raw(&self) -> HANDLE {
        self.raw
    }
}

#[cfg(windows)]
impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        if self.raw != INVALID_HANDLE_VALUE && !self.raw.is_null() {
            unsafe {
                CloseHandle(self.raw);
            }
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct ProcessHandle {
    raw: HANDLE,
}

#[cfg(windows)]
impl ProcessHandle {
    fn open(pid: u32) -> Option<Self> {
        let raw = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };

        let raw = if raw.is_null() {
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }
        } else {
            raw
        };

        (!raw.is_null()).then_some(Self { raw })
    }

    fn raw(&self) -> HANDLE {
        self.raw
    }
}

#[cfg(windows)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                CloseHandle(self.raw);
            }
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct ProcessMemory {
    working_set_bytes: u64,
    private_bytes: u64,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct ProcessIo {
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}

#[cfg(windows)]
fn query_process_image(process: HANDLE) -> Option<String> {
    let mut buffer = vec![0_u16; 32_768];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len) };
    if ok == 0 || len == 0 {
        None
    } else {
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

#[cfg(windows)]
fn query_process_start_time_ms(process: HANDLE) -> Option<u64> {
    let mut creation_time = Default::default();
    let mut exit_time = Default::default();
    let mut kernel_time = Default::default();
    let mut user_time = Default::default();

    let ok = unsafe {
        GetProcessTimes(
            process,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };

    (ok != 0).then(|| filetime_to_unix_ms(creation_time))
}

#[cfg(windows)]
fn query_process_memory(process: HANDLE) -> Option<ProcessMemory> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            process,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    };

    (ok != 0).then(|| ProcessMemory {
        working_set_bytes: usize_to_u64_saturating(counters.WorkingSetSize),
        private_bytes: usize_to_u64_saturating(counters.PrivateUsage),
    })
}

#[cfg(windows)]
fn query_process_io(process: HANDLE) -> Option<ProcessIo> {
    let mut counters = IO_COUNTERS::default();
    let ok = unsafe { GetProcessIoCounters(process, &mut counters) };
    (ok != 0).then_some(ProcessIo {
        read_bytes: counters.ReadTransferCount,
        write_bytes: counters.WriteTransferCount,
        other_bytes: counters.OtherTransferCount,
    })
}

#[cfg(windows)]
fn query_process_handle_count(process: HANDLE) -> Option<u32> {
    let mut handles = 0;
    let ok = unsafe { GetProcessHandleCount(process, &mut handles) };
    (ok != 0).then_some(handles)
}

#[cfg(windows)]
fn wide_null_terminated_to_string(value: &[u16]) -> String {
    let len = value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len])
}

#[cfg(windows)]
fn filetime_to_unix_ms(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    let raw = ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64;
    filetime_100ns_to_unix_ms(raw)
}

pub(crate) fn filetime_100ns_to_unix_ms(value: u64) -> u64 {
    value.saturating_sub(FILETIME_UNIX_EPOCH_100NS) / FILETIME_100NS_PER_MS
}

fn resolve_access_state(successes: usize, failures: usize) -> AccessState {
    if successes == 0 {
        AccessState::Denied
    } else if failures == 0 {
        AccessState::Full
    } else {
        AccessState::Partial
    }
}

#[cfg(windows)]
fn usize_to_u64_saturating(value: usize) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload_row(pid: u32, parent: Option<u32>, start: u64, exe: &str) -> ProcessSample {
        serde_json::from_value(serde_json::json!({
            "pid": pid.to_string(), "parent_pid": parent.map(|pid| pid.to_string()),
            "start_time_ms": start, "name": "BatCave", "exe": exe,
            "status": "running", "cpu_percent": 0.0, "memory_bytes": 0,
            "private_bytes": 0, "io_read_total_bytes": 0, "io_write_total_bytes": 0,
            "io_read_bps": 0, "io_write_bps": 0, "threads": 1, "handles": 0,
            "access_state": "full"
        }))
        .unwrap()
    }

    fn workload_identity(row: &ProcessSample, file_index: u64) -> WorkloadProcessIdentity {
        WorkloadProcessIdentity {
            generation: ProcessGeneration::from_process(row).unwrap(),
            sample_exe: row.exe.clone(),
            image_path: canonical_windows_path_key(&row.exe),
            file_identity: (1, file_index),
            principal_identity: [1; 32],
            session_id: 1,
        }
    }

    fn workload_fixture() -> (
        Vec<ProcessSample>,
        HashMap<usize, WorkloadProcessIdentity>,
        Vec<String>,
    ) {
        let runtime = r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application";
        let helper = format!(r"{runtime}\140.0.3485.81\msedgewebview2.exe");
        let rows = vec![
            workload_row(10, None, 10, r"C:\BatCave\batcave-monitor.exe"),
            workload_row(20, Some(10), 20, &helper),
            workload_row(30, Some(20), 30, &helper),
            workload_row(40, None, 5, r"C:\BatCave\batcave-collector-service.exe"),
            workload_row(50, None, 50, &helper),
            workload_row(60, Some(10), 60, r"C:\Elsewhere\msedgewebview2.exe"),
        ];
        let mut identities = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                (
                    index,
                    workload_identity(row, if index == 0 { 1 } else { 2 }),
                )
            })
            .collect::<HashMap<_, _>>();
        identities.get_mut(&3).unwrap().principal_identity = [8; 32];
        identities.get_mut(&3).unwrap().session_id = 0;
        (rows, identities, vec![canonical_windows_path_key(runtime)])
    }

    #[test]
    fn workload_probe_assembly_keeps_only_owned_helpers_and_authenticated_service() {
        let (rows, identities, roots) = workload_fixture();
        let desktop = identities[&0].generation;
        let service = identities[&3].generation;
        let context =
            context_from_workload_identities(&rows, desktop, Some(service), &identities, &roots)
                .unwrap();
        assert_eq!(
            context
                .members
                .iter()
                .map(|member| member.pid)
                .collect::<Vec<_>>(),
            [10, 20, 30, 40]
        );
        let without_service =
            context_from_workload_identities(&rows, desktop, None, &identities, &roots).unwrap();
        assert_eq!(
            without_service
                .members
                .iter()
                .map(|member| member.pid)
                .collect::<Vec<_>>(),
            [10, 20, 30]
        );
    }

    #[test]
    fn workload_probe_assembly_rejects_user_session_image_and_generation_gaps() {
        let (rows, identities, roots) = workload_fixture();
        for fault in 0..7 {
            let mut identities = identities.clone();
            match fault {
                0 => identities.get_mut(&1).unwrap().principal_identity = [9; 32],
                1 => identities.get_mut(&1).unwrap().session_id = 2,
                2 => identities.get_mut(&1).unwrap().generation.start_time_ms += 1,
                3 => identities
                    .get_mut(&1)
                    .unwrap()
                    .sample_exe
                    .push_str(".changed"),
                4 => identities.get_mut(&1).unwrap().file_identity.1 = 0,
                5 => {
                    identities.get_mut(&1).unwrap().image_path =
                        r"c:\other\msedgewebview2.exe".into()
                }
                _ => {
                    identities.remove(&1);
                }
            }
            let context = context_from_workload_identities(
                &rows,
                identities[&0].generation,
                None,
                &identities,
                &roots,
            )
            .unwrap();
            assert_eq!(
                context.members,
                vec![identities[&0].generation],
                "fault {fault}"
            );
        }
    }

    #[test]
    fn workload_probe_assembly_rejects_duplicates_reused_parents_and_stale_service() {
        let (mut rows, identities, roots) = workload_fixture();
        let desktop = identities[&0].generation;
        let stale_service = ProcessGeneration {
            start_time_ms: 4,
            ..identities[&3].generation
        };
        assert!(!context_from_workload_identities(
            &rows,
            desktop,
            Some(stale_service),
            &identities,
            &roots
        )
        .unwrap()
        .members
        .contains(&identities[&3].generation));
        let mut duplicated_service = rows.clone();
        duplicated_service.push(rows[3].clone());
        assert!(!context_from_workload_identities(
            &duplicated_service,
            desktop,
            Some(identities[&3].generation),
            &identities,
            &roots,
        )
        .unwrap()
        .members
        .contains(&identities[&3].generation));
        let mut duplicated_helper = rows.clone();
        duplicated_helper.push(rows[1].clone());
        assert_eq!(
            context_from_workload_identities(
                &duplicated_helper,
                desktop,
                None,
                &identities,
                &roots,
            )
            .unwrap()
            .members,
            vec![desktop]
        );
        rows[1].start_time_ms = 9;
        let context =
            context_from_workload_identities(&rows, desktop, None, &identities, &roots).unwrap();
        assert_eq!(context.members, vec![desktop]);
        rows.push(rows[0].clone());
        assert!(
            context_from_workload_identities(&rows, desktop, None, &identities, &roots).is_none()
        );
    }

    #[test]
    fn workload_probe_same_image_descendant_requires_the_same_native_file() {
        let rows = vec![
            workload_row(10, None, 10, r"C:\Portable\batcave.exe"),
            workload_row(20, Some(10), 20, r"C:\Portable\batcave.exe"),
        ];
        let mut identities = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (index, workload_identity(row, 1)))
            .collect::<HashMap<_, _>>();
        let desktop = identities[&0].generation;
        assert_eq!(
            context_from_workload_identities(&rows, desktop, None, &identities, &[])
                .unwrap()
                .members
                .len(),
            2
        );
        identities.get_mut(&1).unwrap().file_identity.1 = 2;
        assert_eq!(
            context_from_workload_identities(&rows, desktop, None, &identities, &[])
                .unwrap()
                .members,
            vec![desktop]
        );
    }

    #[test]
    fn workload_probe_helper_paths_require_the_exact_runtime_layout() {
        let roots = vec![r"c:\runtime\application".into()];
        assert!(approved_webview_image(
            r"c:\runtime\application\140.0.1.2\msedgewebview2.exe",
            &roots
        ));
        for path in [
            r"c:\runtime\application-other\140.0.1.2\msedgewebview2.exe",
            r"c:\runtime\application\unknown\msedgewebview2.exe",
            r"c:\runtime\application\140.0.1.2\child\msedgewebview2.exe",
            r"c:\other\140.0.1.2\msedgewebview2.exe",
        ] {
            assert!(!approved_webview_image(path, &roots), "{path}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn workload_probe_binds_the_live_self_row_and_rejects_changed_inputs() {
        let rows = collect_processes(0).expect("native process sample");
        let index = rows
            .iter()
            .position(|row| row.pid.parse::<u32>().ok() == Some(std::process::id()))
            .expect("self row is present");
        let context = batcave_workload_context(&rows, None).expect("native self ownership");
        assert_eq!(
            context.desktop,
            ProcessGeneration::from_process(&rows[index]).unwrap()
        );
        assert!(context.members.contains(&context.desktop));
        for change in 0..3 {
            let mut changed = rows.clone();
            match change {
                0 => changed[index].start_time_ms += 1,
                1 => changed[index].exe.clear(),
                _ => changed.push(rows[index].clone()),
            }
            assert!(
                batcave_workload_context(&changed, None).is_none(),
                "change {change}"
            );
        }
    }

    #[test]
    fn filetime_epoch_converts_to_unix_zero_ms() {
        assert_eq!(filetime_100ns_to_unix_ms(FILETIME_UNIX_EPOCH_100NS), 0);
    }

    #[test]
    fn filetime_after_epoch_converts_to_unix_ms() {
        assert_eq!(
            filetime_100ns_to_unix_ms(FILETIME_UNIX_EPOCH_100NS + 12_345 * FILETIME_100NS_PER_MS),
            12_345
        );
    }

    #[test]
    fn filetime_before_epoch_saturates_to_zero_ms() {
        assert_eq!(filetime_100ns_to_unix_ms(1), 0);
    }

    #[test]
    fn snapshot_fence_rejects_reused_or_ambiguous_same_millisecond_births() {
        assert!(generation_precedes_snapshot(999, 1_000));
        for birth in [0, 1_000, 1_001] {
            assert!(!generation_precedes_snapshot(birth, 1_000));
        }
    }

    #[test]
    fn access_state_requires_all_probes_for_full() {
        assert_eq!(
            resolve_access_state(PROCESS_PROBE_COUNT, 0),
            AccessState::Full
        );
        assert_eq!(resolve_access_state(1, 1), AccessState::Partial);
        assert_eq!(
            resolve_access_state(0, PROCESS_PROBE_COUNT),
            AccessState::Denied
        );
    }

    #[test]
    fn per_field_quality_preserves_successful_windows_probes() {
        let quality = process_metric_quality_from_probes(ProcessProbeResults {
            memory: false,
            io: false,
            threads: true,
            handles: false,
        });

        assert_eq!(
            quality.memory.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.io.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.other_io.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
        assert_eq!(
            quality.threads.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );
        assert_eq!(
            quality.handles.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Unavailable)
        );
    }

    #[cfg(windows)]
    #[test]
    fn collect_processes_includes_current_process_with_native_identity() {
        let current_pid = std::process::id().to_string();
        let processes = collect_processes(0).expect("process collection succeeds");
        let current = processes
            .iter()
            .find(|process| process.pid == current_pid)
            .expect("current process is present in native process snapshot");

        assert!(current.start_time_ms > 0);
        assert!(current.threads > 0);
        assert!(current.handles > 0);
        assert_ne!(current.access_state, AccessState::Denied);
    }
}
