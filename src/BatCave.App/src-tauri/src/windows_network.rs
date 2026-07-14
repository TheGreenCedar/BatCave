#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

use std::collections::HashMap;

use crate::network_attribution::{NetworkAttributionSample, ProcessNetworkRates};

#[cfg(any(windows, test))]
const ETW_CONSUMER_STALL_MS: u64 = 5_000;

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct EtwSessionStatistics {
    events_lost: u64,
    log_buffers_lost: u64,
    realtime_buffers_lost: u64,
}

#[cfg(any(windows, test))]
impl EtwSessionStatistics {
    fn total_loss(self) -> u64 {
        self.events_lost
            .saturating_add(self.log_buffers_lost)
            .saturating_add(self.realtime_buffers_lost)
    }

    fn decreased_from(self, previous: Self) -> bool {
        self.events_lost < previous.events_lost
            || self.log_buffers_lost < previous.log_buffers_lost
            || self.realtime_buffers_lost < previous.realtime_buffers_lost
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct EtwHealthSnapshot {
    consumer_started: bool,
    consumer_heartbeat_age_ms: Option<u64>,
    consumer_error: Option<String>,
    decoded_events: u64,
    decoder_errors: u64,
    session_statistics: Result<EtwSessionStatistics, String>,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum EtwQualityDecision {
    Native,
    PendingBaseline,
    DataLoss(String),
    Unavailable(String),
}

#[cfg(any(windows, test))]
#[derive(Debug, Default)]
struct EtwQualityTracker {
    decoded_events: u64,
    decoder_errors: u64,
    session_statistics: EtwSessionStatistics,
    needs_clean_interval: bool,
}

#[cfg(any(windows, test))]
impl EtwQualityTracker {
    fn evaluate(&mut self, snapshot: EtwHealthSnapshot) -> EtwQualityDecision {
        if let Some(error) = snapshot.consumer_error {
            return EtwQualityDecision::Unavailable(error);
        }

        let statistics = match snapshot.session_statistics {
            Ok(statistics) => statistics,
            Err(error) => return EtwQualityDecision::Unavailable(error),
        };
        if statistics.decreased_from(self.session_statistics) {
            return EtwQualityDecision::Unavailable(
                "network_attribution_session_statistics_regressed".to_string(),
            );
        }

        let decoded_delta = snapshot.decoded_events.saturating_sub(self.decoded_events);
        let decoder_error_delta = snapshot.decoder_errors.saturating_sub(self.decoder_errors);
        let loss_delta = statistics
            .total_loss()
            .saturating_sub(self.session_statistics.total_loss());
        self.decoded_events = snapshot.decoded_events;
        self.decoder_errors = snapshot.decoder_errors;
        self.session_statistics = statistics;

        if decoder_error_delta > 0 || loss_delta > 0 {
            self.needs_clean_interval = true;
            if snapshot.decoded_events == 0 {
                return EtwQualityDecision::Unavailable(format!(
                    "network_attribution_decoder_unproven:decoder_errors={decoder_error_delta}:lost={loss_delta}"
                ));
            }
            return EtwQualityDecision::DataLoss(format!(
                "ETW process-network attribution lost data: decoder_errors={decoder_error_delta}, lost_events_or_buffers={loss_delta}."
            ));
        }

        if !snapshot.consumer_started {
            return EtwQualityDecision::PendingBaseline;
        }
        let Some(heartbeat_age_ms) = snapshot.consumer_heartbeat_age_ms else {
            return EtwQualityDecision::Unavailable(
                "network_attribution_consumer_heartbeat_missing".to_string(),
            );
        };
        if heartbeat_age_ms > ETW_CONSUMER_STALL_MS {
            return EtwQualityDecision::Unavailable(format!(
                "network_attribution_consumer_stalled:{heartbeat_age_ms}ms"
            ));
        }

        if snapshot.decoded_events == 0 {
            return EtwQualityDecision::PendingBaseline;
        }

        if self.needs_clean_interval {
            if decoded_delta == 0 {
                return EtwQualityDecision::DataLoss(
                    "ETW process-network attribution is waiting for a clean decoded interval after data loss."
                        .to_string(),
                );
            }
            self.needs_clean_interval = false;
        }

        EtwQualityDecision::Native
    }
}

#[cfg(windows)]
pub struct NetworkAttributionMonitor {
    inner: WindowsNetworkAttributionMonitor,
}

#[cfg(not(windows))]
pub struct NetworkAttributionMonitor;

impl NetworkAttributionMonitor {
    #[cfg(windows)]
    pub fn new() -> Result<Self, String> {
        WindowsNetworkAttributionMonitor::start().map(|inner| Self { inner })
    }

    #[cfg(not(windows))]
    pub fn new() -> Result<Self, String> {
        Err("network_attribution_requires_windows".to_string())
    }

    #[cfg(windows)]
    pub fn sample(&self) -> NetworkAttributionSample {
        self.inner.sample()
    }

    #[cfg(not(windows))]
    pub fn sample(&self) -> NetworkAttributionSample {
        NetworkAttributionSample::Failed("network_attribution_requires_windows".to_string())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct NetworkByteCounters {
    received_bytes: u64,
    transmitted_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkDirection {
    Received,
    Transmitted,
}

fn apply_network_event(
    counters_by_pid: &mut HashMap<u32, NetworkByteCounters>,
    pid: u32,
    direction: NetworkDirection,
    byte_count: u64,
) {
    if pid == 0 || byte_count == 0 {
        return;
    }

    let counters = counters_by_pid.entry(pid).or_default();
    match direction {
        NetworkDirection::Received => {
            counters.received_bytes = counters.received_bytes.saturating_add(byte_count);
        }
        NetworkDirection::Transmitted => {
            counters.transmitted_bytes = counters.transmitted_bytes.saturating_add(byte_count);
        }
    }
}

fn rate_map_from_deltas(
    current: &HashMap<u32, NetworkByteCounters>,
    previous: &HashMap<u32, NetworkByteCounters>,
    elapsed_seconds: f64,
) -> HashMap<u32, ProcessNetworkRates> {
    current
        .iter()
        .filter_map(|(pid, current)| {
            let previous = previous.get(pid).copied().unwrap_or_default();
            let rates = ProcessNetworkRates {
                received_bps: byte_rate(
                    current.received_bytes,
                    previous.received_bytes,
                    elapsed_seconds,
                ),
                transmitted_bps: byte_rate(
                    current.transmitted_bytes,
                    previous.transmitted_bytes,
                    elapsed_seconds,
                ),
            };
            (rates.received_bps > 0 || rates.transmitted_bps > 0).then_some((*pid, rates))
        })
        .collect()
}

fn byte_rate(current: u64, previous: u64, elapsed_seconds: f64) -> u64 {
    if current < previous {
        return 0;
    }

    ((current - previous) as f64 / elapsed_seconds.max(0.001)).round() as u64
}

fn classify_direction(
    event_name: Option<&str>,
    task_name: Option<&str>,
    opcode_name: Option<&str>,
) -> Option<NetworkDirection> {
    let text = [event_name, task_name, opcode_name]
        .into_iter()
        .flatten()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");

    if text.contains("recv") || text.contains("receive") || text.contains("received") {
        Some(NetworkDirection::Received)
    } else if text.contains("send") || text.contains("sent") || text.contains("transmit") {
        Some(NetworkDirection::Transmitted)
    } else {
        None
    }
}

fn first_matching_property(properties: &HashMap<String, u64>, candidates: &[&str]) -> Option<u64> {
    candidates.iter().find_map(|candidate| {
        properties
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(candidate))
            .map(|(_, value)| *value)
    })
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        collections::HashMap,
        ffi::OsStr,
        mem::{size_of, zeroed},
        os::windows::ffi::OsStrExt,
        ptr::{null, null_mut},
        slice,
        sync::{
            atomic::{AtomicBool, AtomicU64, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::Instant,
    };

    use windows_sys::{
        core::GUID,
        Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
            System::Diagnostics::Etw::{
                CloseTrace, ControlTraceW, OpenTraceW, ProcessTrace, StartTraceW, TcpIpGuid,
                TdhGetEventInformation, TdhGetProperty, TdhGetPropertySize, EVENT_RECORD,
                EVENT_TRACE_CONTROL_QUERY, EVENT_TRACE_CONTROL_STOP,
                EVENT_TRACE_FLAG_NETWORK_TCPIP, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_REAL_TIME_MODE, EVENT_TRACE_SYSTEM_LOGGER_MODE, PROCESSTRACE_HANDLE,
                PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME,
                PROPERTY_DATA_DESCRIPTOR, TRACE_EVENT_INFO, WNODE_FLAG_TRACED_GUID,
            },
        },
    };

    use super::{
        apply_network_event, classify_direction, first_matching_property, rate_map_from_deltas,
        EtwHealthSnapshot, EtwQualityDecision, EtwQualityTracker, EtwSessionStatistics,
        NetworkAttributionSample, NetworkByteCounters, NetworkDirection,
    };

    const INVALID_PROCESSTRACE_HANDLE: u64 = u64::MAX;
    const LOGGER_NAME: &str = "BatCave Process Network";
    const LOGGER_GUID: GUID = GUID {
        data1: 0x4cbda9c4,
        data2: 0x64aa,
        data3: 0x4c39,
        data4: [0x91, 0xd3, 0x45, 0x1d, 0xb1, 0x87, 0x54, 0xe7],
    };
    const PROPERTY_SIZE_NAMES: [&str; 6] = [
        "size",
        "Size",
        "payload_size",
        "PayloadSize",
        "packet_size",
        "PacketSize",
    ];
    const PROPERTY_PID_NAMES: [&str; 4] = ["PID", "Pid", "ProcessId", "process_id"];

    pub struct WindowsNetworkAttributionMonitor {
        shared: Arc<NetworkEtwShared>,
        trace_handle: u64,
        process_thread: Option<JoinHandle<()>>,
        session_name: Vec<u16>,
        previous_sample_at: Mutex<Instant>,
        quality: Mutex<EtwQualityTracker>,
    }

    impl WindowsNetworkAttributionMonitor {
        pub fn start() -> Result<Self, String> {
            let session_name = wide(LOGGER_NAME);
            let mut properties = trace_properties(&session_name);
            let mut trace_handle = Default::default();
            let start_result = unsafe {
                StartTraceW(
                    &mut trace_handle,
                    session_name.as_ptr(),
                    properties.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                )
            };

            let trace_handle = trace_handle_from_start_result(start_result, trace_handle.Value)?;

            let shared = Arc::new(NetworkEtwShared::new());
            let process_handle = match open_trace(&session_name, &shared) {
                Ok(handle) => handle,
                Err(error) => {
                    stop_trace(trace_handle, &session_name);
                    return Err(error);
                }
            };
            let thread_shared = Arc::clone(&shared);
            let process_thread = thread::Builder::new()
                .name("batcave-network-etw".to_string())
                .spawn(move || process_trace_loop(process_handle, thread_shared))
                .map_err(|error| {
                    stop_trace(trace_handle, &session_name);
                    format!("network_attribution_thread_start_failed:{error}")
                })?;

            Ok(Self {
                shared,
                trace_handle,
                process_thread: Some(process_thread),
                session_name,
                previous_sample_at: Mutex::new(Instant::now()),
                quality: Mutex::new(EtwQualityTracker::default()),
            })
        }

        pub fn sample(&self) -> NetworkAttributionSample {
            let current = self.shared.drain_counters();
            let now = Instant::now();
            let mut previous_sample_at = match self.previous_sample_at.lock() {
                Ok(value) => value,
                Err(_) => {
                    return NetworkAttributionSample::Failed(
                        "network_attribution_sample_clock_lock_poisoned".to_string(),
                    )
                }
            };
            let elapsed_seconds = now.duration_since(*previous_sample_at).as_secs_f64();
            *previous_sample_at = now;

            let rates_by_pid = rate_map_from_deltas(&current, &HashMap::new(), elapsed_seconds);
            let session_statistics = query_trace_statistics(self.trace_handle, &self.session_name);
            let snapshot = EtwHealthSnapshot {
                consumer_started: self.shared.consumer_started(),
                consumer_heartbeat_age_ms: match self.shared.consumer_heartbeat_age_ms() {
                    Ok(age) => age,
                    Err(error) => return NetworkAttributionSample::Failed(error),
                },
                consumer_error: self.shared.last_error(),
                decoded_events: self.shared.decoded_events(),
                decoder_errors: self.shared.decoder_errors(),
                session_statistics,
            };
            let decision = match self.quality.lock() {
                Ok(mut quality) => quality.evaluate(snapshot),
                Err(_) => {
                    return NetworkAttributionSample::Failed(
                        "network_attribution_quality_lock_poisoned".to_string(),
                    )
                }
            };

            match decision {
                EtwQualityDecision::Native => NetworkAttributionSample::Ready { rates_by_pid },
                EtwQualityDecision::PendingBaseline => NetworkAttributionSample::PendingBaseline(
                    "Waiting for a supported ETW process-network event baseline.".to_string(),
                ),
                EtwQualityDecision::DataLoss(message) => NetworkAttributionSample::Partial {
                    rates_by_pid,
                    message,
                },
                EtwQualityDecision::Unavailable(message) => {
                    NetworkAttributionSample::Failed(message)
                }
            }
        }
    }

    fn trace_handle_from_start_result(start_result: u32, trace_handle: u64) -> Result<u64, String> {
        match start_result {
            ERROR_SUCCESS => Ok(trace_handle),
            ERROR_ALREADY_EXISTS => Err("network_attribution_existing_trace_session".to_string()),
            _ => Err(format!(
                "network_attribution_start_trace_failed:{start_result}"
            )),
        }
    }

    impl Drop for WindowsNetworkAttributionMonitor {
        fn drop(&mut self) {
            stop_trace(self.trace_handle, &self.session_name);
            if let Some(thread) = self.process_thread.take() {
                let _ = thread.join();
            }
        }
    }

    struct NetworkEtwShared {
        counters_by_pid: Mutex<HashMap<u32, NetworkByteCounters>>,
        error: Mutex<Option<String>>,
        decoded_events: AtomicU64,
        decoder_errors: AtomicU64,
        consumer_started: AtomicBool,
        consumer_started_at: Instant,
        consumer_heartbeat_at: Mutex<Option<Instant>>,
    }

    impl NetworkEtwShared {
        fn new() -> Self {
            Self {
                counters_by_pid: Mutex::new(HashMap::new()),
                error: Mutex::new(None),
                decoded_events: AtomicU64::new(0),
                decoder_errors: AtomicU64::new(0),
                consumer_started: AtomicBool::new(false),
                consumer_started_at: Instant::now(),
                consumer_heartbeat_at: Mutex::new(None),
            }
        }

        fn record(&self, event: NetworkEtwEvent) {
            if let Ok(mut counters) = self.counters_by_pid.lock() {
                apply_network_event(&mut counters, event.pid, event.direction, event.byte_count);
                self.decoded_events.fetch_add(1, Ordering::Relaxed);
            }
        }

        fn record_decoder_error(&self) {
            self.decoder_errors.fetch_add(1, Ordering::Relaxed);
        }

        fn record_consumer_heartbeat(&self) {
            if let Ok(mut heartbeat) = self.consumer_heartbeat_at.lock() {
                *heartbeat = Some(Instant::now());
            }
        }

        fn drain_counters(&self) -> HashMap<u32, NetworkByteCounters> {
            self.counters_by_pid
                .lock()
                .map(|mut value| std::mem::take(&mut *value))
                .unwrap_or_default()
        }

        fn set_error(&self, error: String) {
            if let Ok(mut current) = self.error.lock() {
                *current = Some(error);
            }
        }

        fn last_error(&self) -> Option<String> {
            self.error.lock().ok().and_then(|value| value.clone())
        }

        fn mark_consumer_started(&self) {
            self.consumer_started.store(true, Ordering::Release);
            self.record_consumer_heartbeat();
        }

        fn consumer_started(&self) -> bool {
            self.consumer_started.load(Ordering::Acquire)
        }

        fn consumer_heartbeat_age_ms(&self) -> Result<Option<u64>, String> {
            let heartbeat = self
                .consumer_heartbeat_at
                .lock()
                .map_err(|_| "network_attribution_consumer_heartbeat_lock_poisoned".to_string())?;
            let observed_at = if self.consumer_started() {
                heartbeat.unwrap_or(self.consumer_started_at)
            } else {
                return Ok(None);
            };
            Ok(Some(
                u64::try_from(observed_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            ))
        }

        fn decoded_events(&self) -> u64 {
            self.decoded_events.load(Ordering::Relaxed)
        }

        fn decoder_errors(&self) -> u64 {
            self.decoder_errors.load(Ordering::Relaxed)
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct NetworkEtwEvent {
        pid: u32,
        direction: NetworkDirection,
        byte_count: u64,
    }

    unsafe extern "system" fn event_record_callback(event_record: *mut EVENT_RECORD) {
        if event_record.is_null() {
            return;
        }
        let shared = (*event_record).UserContext as *const NetworkEtwShared;
        if shared.is_null() {
            return;
        }
        (*shared).record_consumer_heartbeat();
        if !same_guid(&(*event_record).EventHeader.ProviderId, &TcpIpGuid) {
            return;
        }

        match decode_network_event(event_record) {
            NetworkEtwDecode::Decoded(event) => (*shared).record(event),
            NetworkEtwDecode::Ignored => {}
            NetworkEtwDecode::Failed => (*shared).record_decoder_error(),
        }
    }

    fn process_trace_loop(process_handle: PROCESSTRACE_HANDLE, shared: Arc<NetworkEtwShared>) {
        shared.mark_consumer_started();
        let handles = [process_handle];
        let result = unsafe { ProcessTrace(handles.as_ptr(), 1, null(), null()) };
        shared.set_error(format!("network_attribution_process_trace_ended:{result}"));
        unsafe {
            CloseTrace(process_handle);
        }
    }

    fn open_trace(
        session_name: &[u16],
        shared: &Arc<NetworkEtwShared>,
    ) -> Result<PROCESSTRACE_HANDLE, String> {
        let mut logfile = EVENT_TRACE_LOGFILEW {
            LoggerName: session_name.as_ptr() as *mut u16,
            Anonymous1: unsafe { zeroed() },
            Anonymous2: unsafe { zeroed() },
            Context: Arc::as_ptr(shared) as *mut _,
            ..Default::default()
        };
        logfile.Anonymous1.ProcessTraceMode =
            PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        logfile.Anonymous2.EventRecordCallback = Some(event_record_callback);
        logfile.BufferCallback = Some(buffer_callback);

        let handle = unsafe { OpenTraceW(&mut logfile) };
        if handle.Value == INVALID_PROCESSTRACE_HANDLE {
            Err("network_attribution_open_trace_failed".to_string())
        } else {
            Ok(handle)
        }
    }

    unsafe extern "system" fn buffer_callback(logfile: *mut EVENT_TRACE_LOGFILEW) -> u32 {
        if logfile.is_null() {
            return 0;
        }
        let shared = (*logfile).Context as *const NetworkEtwShared;
        if shared.is_null() {
            return 0;
        }
        (*shared).record_consumer_heartbeat();
        1
    }

    fn stop_trace(trace_handle: u64, session_name: &[u16]) {
        if trace_handle == 0 {
            return;
        }
        let mut properties = trace_properties(session_name);
        unsafe {
            ControlTraceW(
                windows_sys::Win32::System::Diagnostics::Etw::CONTROLTRACE_HANDLE {
                    Value: trace_handle,
                },
                session_name.as_ptr(),
                properties.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_CONTROL_STOP,
            );
        }
    }

    fn query_trace_statistics(
        trace_handle: u64,
        session_name: &[u16],
    ) -> Result<EtwSessionStatistics, String> {
        let mut properties = trace_properties(session_name);
        let result = unsafe {
            ControlTraceW(
                windows_sys::Win32::System::Diagnostics::Etw::CONTROLTRACE_HANDLE {
                    Value: trace_handle,
                },
                session_name.as_ptr(),
                properties.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_CONTROL_QUERY,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(format!("network_attribution_query_trace_failed:{result}"));
        }

        let properties = unsafe { &*(properties.as_ptr() as *const EVENT_TRACE_PROPERTIES) };
        if !same_guid(&properties.Wnode.Guid, &LOGGER_GUID)
            || properties.EnableFlags & EVENT_TRACE_FLAG_NETWORK_TCPIP
                != EVENT_TRACE_FLAG_NETWORK_TCPIP
            || properties.LogFileMode & EVENT_TRACE_REAL_TIME_MODE == 0
            || properties.LogFileMode & EVENT_TRACE_SYSTEM_LOGGER_MODE == 0
        {
            return Err("network_attribution_session_configuration_changed".to_string());
        }

        Ok(EtwSessionStatistics {
            events_lost: u64::from(properties.EventsLost),
            log_buffers_lost: u64::from(properties.LogBuffersLost),
            realtime_buffers_lost: u64::from(properties.RealTimeBuffersLost),
        })
    }

    fn trace_properties(session_name: &[u16]) -> Vec<u64> {
        let properties_size = size_of::<EVENT_TRACE_PROPERTIES>();
        let name_bytes = std::mem::size_of_val(session_name);
        let buffer_bytes = properties_size + name_bytes;
        let mut buffer = vec![0_u64; buffer_bytes.div_ceil(size_of::<u64>())];
        unsafe {
            let properties = &mut *(buffer.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
            properties.Wnode.BufferSize = buffer_bytes as u32;
            properties.Wnode.Guid = LOGGER_GUID;
            properties.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            properties.BufferSize = 64;
            properties.MinimumBuffers = 4;
            properties.MaximumBuffers = 32;
            properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE;
            properties.FlushTimer = 1;
            properties.EnableFlags = EVENT_TRACE_FLAG_NETWORK_TCPIP;
            properties.LoggerNameOffset = properties_size as u32;

            let name_target = (buffer.as_mut_ptr() as *mut u8).add(properties_size) as *mut u16;
            std::ptr::copy_nonoverlapping(session_name.as_ptr(), name_target, session_name.len());
        }

        buffer
    }

    enum NetworkEtwDecode {
        Decoded(NetworkEtwEvent),
        Ignored,
        Failed,
    }

    fn decode_network_event(event_record: *mut EVENT_RECORD) -> NetworkEtwDecode {
        let Some(metadata) = read_event_metadata(event_record) else {
            return NetworkEtwDecode::Failed;
        };
        let event_name = metadata.string_at(metadata.event_name_offset);
        let task_name = metadata.string_at(metadata.task_name_offset);
        let opcode_name = metadata.string_at(metadata.opcode_name_offset);
        let Some(direction) = classify_direction(
            event_name.as_deref(),
            task_name.as_deref(),
            opcode_name.as_deref(),
        ) else {
            return NetworkEtwDecode::Ignored;
        };

        let properties = metadata.numeric_properties(event_record);
        let Some(byte_count) = first_matching_property(&properties, &PROPERTY_SIZE_NAMES) else {
            return NetworkEtwDecode::Failed;
        };
        let pid = first_matching_property(&properties, &PROPERTY_PID_NAMES)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_else(|| unsafe { (*event_record).EventHeader.ProcessId });
        if pid == 0 || byte_count == 0 {
            return NetworkEtwDecode::Failed;
        }

        NetworkEtwDecode::Decoded(NetworkEtwEvent {
            pid,
            direction,
            byte_count,
        })
    }

    struct EventMetadata {
        buffer: Vec<u64>,
        buffer_size: usize,
        event_name_offset: u32,
        task_name_offset: u32,
        opcode_name_offset: u32,
        properties: Vec<(String, String)>,
    }

    impl EventMetadata {
        fn string_at(&self, offset: u32) -> Option<String> {
            wide_string_at(self.bytes(), offset)
        }

        fn bytes(&self) -> &[u8] {
            unsafe { slice::from_raw_parts(self.buffer.as_ptr() as *const u8, self.buffer_size) }
        }

        fn numeric_properties(&self, event_record: *mut EVENT_RECORD) -> HashMap<String, u64> {
            self.properties
                .iter()
                .filter_map(|(name, lookup_name)| {
                    read_property_u64(event_record, lookup_name).map(|value| (name.clone(), value))
                })
                .collect()
        }
    }

    fn read_event_metadata(event_record: *mut EVENT_RECORD) -> Option<EventMetadata> {
        let mut buffer_size = 0_u32;
        let sizing = unsafe {
            TdhGetEventInformation(event_record, 0, null(), null_mut(), &mut buffer_size)
        };
        if sizing != ERROR_INSUFFICIENT_BUFFER || buffer_size == 0 {
            return None;
        }

        let requested_size = buffer_size as usize;
        let mut buffer = vec![0_u64; requested_size.div_ceil(size_of::<u64>())];
        let status = unsafe {
            TdhGetEventInformation(
                event_record,
                0,
                null(),
                buffer.as_mut_ptr() as *mut TRACE_EVENT_INFO,
                &mut buffer_size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }

        let info = unsafe { &*(buffer.as_ptr() as *const TRACE_EVENT_INFO) };
        let event_name_offset = unsafe { info.Anonymous1.EventNameOffset };
        let task_name_offset = info.TaskNameOffset;
        let opcode_name_offset = info.OpcodeNameOffset;
        let property_count = info.TopLevelPropertyCount as usize;
        let first_property = info.EventPropertyInfoArray.as_ptr();
        let properties = unsafe { slice::from_raw_parts(first_property, property_count) }
            .iter()
            .filter_map(|property| {
                let bytes =
                    unsafe { slice::from_raw_parts(buffer.as_ptr() as *const u8, requested_size) };
                wide_string_at(bytes, property.NameOffset).map(|name| (name.clone(), name))
            })
            .collect();

        Some(EventMetadata {
            buffer,
            buffer_size: requested_size,
            event_name_offset,
            task_name_offset,
            opcode_name_offset,
            properties,
        })
    }

    fn read_property_u64(event_record: *mut EVENT_RECORD, property_name: &str) -> Option<u64> {
        let name = wide(property_name);
        let descriptor = PROPERTY_DATA_DESCRIPTOR {
            PropertyName: name.as_ptr() as u64,
            ArrayIndex: u32::MAX,
            Reserved: 0,
        };
        let mut property_size = 0_u32;
        let size_status = unsafe {
            TdhGetPropertySize(event_record, 0, null(), 1, &descriptor, &mut property_size)
        };
        if size_status != ERROR_SUCCESS || property_size == 0 || property_size > 16 {
            return None;
        }

        let mut buffer = vec![0_u8; property_size as usize];
        let status = unsafe {
            TdhGetProperty(
                event_record,
                0,
                null(),
                1,
                &descriptor,
                property_size,
                buffer.as_mut_ptr(),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }

        numeric_le_bytes_to_u64(&buffer)
    }

    fn numeric_le_bytes_to_u64(value: &[u8]) -> Option<u64> {
        match value.len() {
            1 => Some(value[0] as u64),
            2 => Some(u16::from_le_bytes([value[0], value[1]]) as u64),
            4 => Some(u32::from_le_bytes([value[0], value[1], value[2], value[3]]) as u64),
            8 => Some(u64::from_le_bytes([
                value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
            ])),
            _ => None,
        }
    }

    fn wide_string_at(buffer: &[u8], offset: u32) -> Option<String> {
        if offset == 0 || offset as usize >= buffer.len() {
            return None;
        }
        let start = offset as usize;
        let remaining = &buffer[start..];
        let values = remaining
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .take_while(|value| *value != 0)
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| String::from_utf16_lossy(&values))
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn same_guid(left: &GUID, right: &GUID) -> bool {
        left.data1 == right.data1
            && left.data2 == right.data2
            && left.data3 == right.data3
            && left.data4 == right.data4
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn existing_trace_session_fails_before_process_thread_starts() {
            assert_eq!(
                trace_handle_from_start_result(ERROR_ALREADY_EXISTS, 0),
                Err("network_attribution_existing_trace_session".to_string())
            );
        }

        #[test]
        fn numeric_le_bytes_cover_common_etw_integer_widths() {
            assert_eq!(numeric_le_bytes_to_u64(&[7]), Some(7));
            assert_eq!(numeric_le_bytes_to_u64(&[7, 1]), Some(263));
            assert_eq!(numeric_le_bytes_to_u64(&[1, 0, 0, 0]), Some(1));
            assert_eq!(numeric_le_bytes_to_u64(&[1, 0, 0, 0, 0, 0, 0, 0]), Some(1));
            assert_eq!(numeric_le_bytes_to_u64(&[1, 2, 3]), None);
        }
    }
}

#[cfg(windows)]
use windows_impl::WindowsNetworkAttributionMonitor;

#[cfg(test)]
mod tests {
    use super::*;

    fn health(
        consumer_started: bool,
        decoded_events: u64,
        decoder_errors: u64,
        session_statistics: Result<EtwSessionStatistics, String>,
    ) -> EtwHealthSnapshot {
        EtwHealthSnapshot {
            consumer_started,
            consumer_heartbeat_age_ms: consumer_started.then_some(0),
            consumer_error: None,
            decoded_events,
            decoder_errors,
            session_statistics,
        }
    }

    #[test]
    fn etw_quality_requires_a_supported_decoded_event_before_native_zero() {
        let mut quality = EtwQualityTracker::default();

        assert_eq!(
            quality.evaluate(health(true, 0, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::PendingBaseline
        );
        assert_eq!(
            quality.evaluate(health(true, 1, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::Native
        );
        assert_eq!(
            quality.evaluate(health(true, 1, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::Native
        );
    }

    #[test]
    fn etw_quality_fails_closed_when_decoder_proof_never_succeeds() {
        let mut quality = EtwQualityTracker::default();

        let decision = quality.evaluate(health(true, 0, 1, Ok(EtwSessionStatistics::default())));

        assert!(
            matches!(decision, EtwQualityDecision::Unavailable(message) if message.contains("decoder_unproven"))
        );

        let mut quality = EtwQualityTracker::default();
        let loss = EtwSessionStatistics {
            realtime_buffers_lost: 1,
            ..EtwSessionStatistics::default()
        };
        let decision = quality.evaluate(health(false, 0, 0, Ok(loss)));
        assert!(
            matches!(decision, EtwQualityDecision::Unavailable(message) if message.contains("lost=1"))
        );
    }

    #[test]
    fn etw_quality_requires_a_clean_decoded_interval_after_loss() {
        let mut quality = EtwQualityTracker::default();
        assert_eq!(
            quality.evaluate(health(true, 1, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::Native
        );

        let loss = EtwSessionStatistics {
            events_lost: 2,
            ..EtwSessionStatistics::default()
        };
        assert!(matches!(
            quality.evaluate(health(true, 2, 0, Ok(loss))),
            EtwQualityDecision::DataLoss(_)
        ));
        assert!(matches!(
            quality.evaluate(health(true, 2, 0, Ok(loss))),
            EtwQualityDecision::DataLoss(_)
        ));
        assert_eq!(
            quality.evaluate(health(true, 3, 0, Ok(loss))),
            EtwQualityDecision::Native
        );
    }

    #[test]
    fn etw_quality_fails_closed_on_consumer_or_session_failure() {
        let mut quality = EtwQualityTracker::default();
        let mut consumer_failed = health(true, 1, 0, Ok(EtwSessionStatistics::default()));
        consumer_failed.consumer_error = Some("network_attribution_process_trace_ended:0".into());
        assert_eq!(
            quality.evaluate(consumer_failed),
            EtwQualityDecision::Unavailable(
                "network_attribution_process_trace_ended:0".to_string()
            )
        );

        assert_eq!(
            quality.evaluate(health(
                true,
                1,
                0,
                Err("network_attribution_query_trace_failed:5".to_string())
            )),
            EtwQualityDecision::Unavailable("network_attribution_query_trace_failed:5".to_string())
        );
    }

    #[test]
    fn etw_quality_rejects_a_stalled_consumer_and_restarts_unproven() {
        let mut quality = EtwQualityTracker::default();
        assert_eq!(
            quality.evaluate(health(true, 1, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::Native
        );

        let mut stalled = health(true, 1, 0, Ok(EtwSessionStatistics::default()));
        stalled.consumer_heartbeat_age_ms = Some(ETW_CONSUMER_STALL_MS + 1);
        assert!(matches!(
            quality.evaluate(stalled),
            EtwQualityDecision::Unavailable(message) if message.contains("consumer_stalled")
        ));

        let mut restarted = EtwQualityTracker::default();
        assert_eq!(
            restarted.evaluate(health(true, 0, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::PendingBaseline
        );
        assert_eq!(
            restarted.evaluate(health(true, 1, 0, Ok(EtwSessionStatistics::default()))),
            EtwQualityDecision::Native
        );
    }

    #[test]
    fn network_events_accumulate_by_pid_and_direction() {
        let mut counters = HashMap::new();

        apply_network_event(&mut counters, 42, NetworkDirection::Received, 128);
        apply_network_event(&mut counters, 42, NetworkDirection::Transmitted, 64);
        apply_network_event(&mut counters, 42, NetworkDirection::Received, 256);
        apply_network_event(&mut counters, 0, NetworkDirection::Received, 512);

        assert_eq!(
            counters.get(&42),
            Some(&NetworkByteCounters {
                received_bytes: 384,
                transmitted_bytes: 64,
            })
        );
        assert!(!counters.contains_key(&0));
    }

    #[test]
    fn rates_are_derived_from_byte_counter_deltas() {
        let previous = HashMap::from([(
            42,
            NetworkByteCounters {
                received_bytes: 1000,
                transmitted_bytes: 2000,
            },
        )]);
        let current = HashMap::from([(
            42,
            NetworkByteCounters {
                received_bytes: 2024,
                transmitted_bytes: 2512,
            },
        )]);

        let rates = rate_map_from_deltas(&current, &previous, 2.0);

        assert_eq!(
            rates.get(&42),
            Some(&ProcessNetworkRates {
                received_bps: 512,
                transmitted_bps: 256,
            })
        );
    }

    #[test]
    fn network_direction_uses_kernel_event_names() {
        assert_eq!(
            classify_direction(None, None, Some("Data received.")),
            Some(NetworkDirection::Received)
        );
        assert_eq!(
            classify_direction(None, None, Some("Data sent.")),
            Some(NetworkDirection::Transmitted)
        );
        assert_eq!(classify_direction(Some("Connection"), None, None), None);
    }

    #[test]
    fn property_lookup_is_case_insensitive() {
        let properties = HashMap::from([("PID".to_string(), 42), ("size".to_string(), 128)]);

        assert_eq!(first_matching_property(&properties, &["pid"]), Some(42));
        assert_eq!(first_matching_property(&properties, &["Size"]), Some(128));
    }
}
