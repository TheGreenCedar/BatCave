#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use crate::contracts::{AccessState, ProcessSample};

const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
const FILETIME_100NS_PER_MS: u64 = 10_000;
const PROCESS_PROBE_COUNT: usize = 5;

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
        processes.push(sample_from_entry(&entry));

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
fn sample_from_entry(entry: &PROCESSENTRY32W) -> ProcessSample {
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
        status: "running".to_string(),
        cpu_percent: 0.0,
        kernel_cpu_percent: None,
        memory_bytes: 0,
        private_bytes: 0,
        virtual_memory_bytes: 0,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        other_io_total_bytes: None,
        disk_read_bps: 0,
        disk_write_bps: 0,
        other_io_bps: None,
        network_received_bps: None,
        network_transmitted_bps: None,
        threads: entry.cntThreads,
        handles: 0,
        access_state: AccessState::Denied,
        quality: None,
    };

    let Some(process) = ProcessHandle::open(pid) else {
        sample.access_state = resolve_access_state(0, PROCESS_PROBE_COUNT);
        return sample;
    };

    let mut succeeded = 0;
    let mut failed = 0;

    match query_process_image(process.raw()) {
        Some(exe) => {
            sample.exe = exe;
            succeeded += 1;
        }
        None => failed += 1,
    }

    match query_process_start_time_ms(process.raw()) {
        Some(start_time_ms) => {
            sample.start_time_ms = start_time_ms;
            succeeded += 1;
        }
        None => failed += 1,
    }

    match query_process_memory(process.raw()) {
        Some(memory) => {
            sample.memory_bytes = memory.working_set_bytes;
            sample.private_bytes = memory.private_bytes;
            sample.virtual_memory_bytes = memory.virtual_memory_bytes;
            succeeded += 1;
        }
        None => failed += 1,
    }

    match query_process_io(process.raw()) {
        Some(io) => {
            sample.disk_read_total_bytes = io.read_bytes;
            sample.disk_write_total_bytes = io.write_bytes;
            sample.other_io_total_bytes = Some(io.other_bytes);
            succeeded += 1;
        }
        None => failed += 1,
    }

    match query_process_handle_count(process.raw()) {
        Some(handles) => {
            sample.handles = handles;
            succeeded += 1;
        }
        None => failed += 1,
    }

    sample.access_state = resolve_access_state(succeeded, failed);
    sample
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
    virtual_memory_bytes: u64,
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
        virtual_memory_bytes: usize_to_u64_saturating(counters.PagefileUsage),
    })
}

#[cfg(windows)]
fn query_process_io(process: HANDLE) -> Option<ProcessIo> {
    let mut counters = IO_COUNTERS::default();
    let ok = unsafe { GetProcessIoCounters(process, &mut counters) };
    (ok != 0).then(|| ProcessIo {
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

fn filetime_100ns_to_unix_ms(value: u64) -> u64 {
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
