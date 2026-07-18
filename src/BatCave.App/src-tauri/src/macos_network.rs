use std::{
    collections::HashMap,
    ffi::CStr,
    io,
    mem::{size_of, zeroed},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::network_attribution::{
    NetworkAttributionSample, ObservedProcessGeneration, ProcessNetworkRates,
};

// Minimal client for Apple's APSL-licensed XNU NStat wire protocol. BatCave does
// not link a private framework or copy XNU implementation code. These message
// layouts and descriptor offsets are qualified against ntstat.h revision 9 in
// XNU tags 8019.41.5 through 12377.1.9 (Darwin 21-25). Fail closed elsewhere.
// Source: https://github.com/apple-oss-distributions/xnu/blob/xnu-12377.1.9/bsd/net/ntstat.h
const CONTROL_NAME: &[u8] = b"com.apple.network.statistics\0";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const SOCKET_POLL_TIMEOUT_MS: i32 = 250;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_MESSAGE_SIZE: usize = 65_536;
const RECEIVE_BUFFER_SIZE: libc::c_int = 1_048_576;

const NSTAT_MSG_TYPE_SUCCESS: u32 = 0;
const NSTAT_MSG_TYPE_ERROR: u32 = 1;
const NSTAT_MSG_TYPE_ADD_ALL_SRCS: u32 = 1002;
const NSTAT_MSG_TYPE_GET_UPDATE: u32 = 1007;
const NSTAT_MSG_TYPE_SRC_ADDED: u32 = 10_001;
const NSTAT_MSG_TYPE_SRC_REMOVED: u32 = 10_002;
const NSTAT_MSG_TYPE_SRC_UPDATE: u32 = 10_006;
const NSTAT_MSG_TYPE_SRC_EXTENDED_UPDATE: u32 = 10_007;

const NSTAT_MSG_HDR_FLAG_CONTINUATION: u16 = 1 << 0;
const NSTAT_MSG_HDR_FLAG_CLOSING: u16 = 1 << 2;
const NSTAT_EVENT_SRC_PREV_EVENT_DISCARDED: u64 = 0x8000_0000;
const NSTAT_FILTER_PROVIDER_NOZEROBYTES: u64 = 0x0040_0000;
const NSTAT_FILTER_PROVIDER_NOZERODELTAS: u64 = 0x0080_0000;
const NSTAT_SRC_REF_ALL: u64 = u64::MAX;

const NSTAT_PROVIDER_TCP_KERNEL: u32 = 2;
const NSTAT_PROVIDER_TCP_USERLAND: u32 = 3;
const NSTAT_PROVIDER_UDP_KERNEL: u32 = 4;
const NSTAT_PROVIDER_UDP_USERLAND: u32 = 5;
const NSTAT_PROVIDER_QUIC_USERLAND: u32 = 8;
const PROVIDERS: [u32; 5] = [
    NSTAT_PROVIDER_TCP_KERNEL,
    NSTAT_PROVIDER_TCP_USERLAND,
    NSTAT_PROVIDER_UDP_KERNEL,
    NSTAT_PROVIDER_UDP_USERLAND,
    NSTAT_PROVIDER_QUIC_USERLAND,
];

const HEADER_LEN: usize = 16;
const UPDATE_PREFIX_LEN: usize = 152;
const UPDATE_SOURCE_REF_OFFSET: usize = 16;
const UPDATE_RX_BYTES_OFFSET: usize = 40;
const UPDATE_TX_BYTES_OFFSET: usize = 56;
const UPDATE_PROVIDER_OFFSET: usize = 144;
const TCP_DESCRIPTOR_PID_OFFSET: usize = 116;
const UDP_DESCRIPTOR_PID_OFFSET: usize = 128;
const MIN_QUALIFIED_DARWIN_MAJOR: u32 = 21;
const MAX_QUALIFIED_DARWIN_MAJOR: u32 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MessageHeader {
    context: u64,
    message_type: u32,
    length: usize,
    flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceUpdate {
    source_ref: u64,
    pid: u32,
    unique_pid: u64,
    received_bytes: u64,
    transmitted_bytes: u64,
    closing: bool,
    previous_event_discarded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceState {
    pid: u32,
    unique_pid: u64,
    received_bytes: u64,
    transmitted_bytes: u64,
}

#[derive(Debug)]
struct SharedSample {
    baseline_complete: bool,
    interval_bytes_by_process: HashMap<ObservedProcessGeneration, ProcessNetworkRates>,
    interval_started_at: Instant,
    failure: Option<String>,
    data_loss: Option<String>,
}

impl Default for SharedSample {
    fn default() -> Self {
        Self {
            baseline_complete: false,
            interval_bytes_by_process: HashMap::new(),
            interval_started_at: Instant::now(),
            failure: None,
            data_loss: None,
        }
    }
}

#[derive(Debug, Default)]
struct AttributionEngine {
    sources: HashMap<u64, SourceState>,
    baseline_complete: bool,
}

impl AttributionEngine {
    fn apply_update(
        &mut self,
        update: SourceUpdate,
        interval: &mut HashMap<ObservedProcessGeneration, ProcessNetworkRates>,
    ) -> Option<String> {
        let previous = self.sources.get(&update.source_ref).copied();
        let mut data_loss = update.previous_event_discarded.then(|| {
            format!(
                "nstat_previous_event_discarded:source_ref={}",
                update.source_ref
            )
        });

        if let Some(previous) = previous {
            if update.pid != previous.pid || update.unique_pid != previous.unique_pid {
                data_loss = Some(format!(
                    "nstat_source_generation_changed:source_ref={}:previous_unique_pid={}:current_unique_pid={}",
                    update.source_ref, previous.unique_pid, update.unique_pid
                ));
            } else if update.received_bytes < previous.received_bytes
                || update.transmitted_bytes < previous.transmitted_bytes
            {
                data_loss = Some(format!(
                    "nstat_counter_regressed:source_ref={}:unique_pid={}",
                    update.source_ref, previous.unique_pid
                ));
            } else {
                let rates = interval
                    .entry(ObservedProcessGeneration::platform(
                        previous.pid,
                        previous.unique_pid,
                    ))
                    .or_default();
                rates.received_bps = rates
                    .received_bps
                    .saturating_add(update.received_bytes - previous.received_bytes);
                rates.transmitted_bps = rates
                    .transmitted_bps
                    .saturating_add(update.transmitted_bytes - previous.transmitted_bytes);
            }
        } else if self.baseline_complete {
            let rates = interval
                .entry(ObservedProcessGeneration::platform(
                    update.pid,
                    update.unique_pid,
                ))
                .or_default();
            rates.received_bps = rates.received_bps.saturating_add(update.received_bytes);
            rates.transmitted_bps = rates
                .transmitted_bps
                .saturating_add(update.transmitted_bytes);
        }

        if update.closing {
            self.sources.remove(&update.source_ref);
        } else {
            self.sources.insert(
                update.source_ref,
                SourceState {
                    pid: update.pid,
                    unique_pid: update.unique_pid,
                    received_bytes: update.received_bytes,
                    transmitted_bytes: update.transmitted_bytes,
                },
            );
        }
        data_loss
    }

    fn finish_baseline(&mut self) {
        self.baseline_complete = true;
    }

    fn remove_source(&mut self, source_ref: u64) {
        self.sources.remove(&source_ref);
    }
}

pub struct MacosNetworkAttribution {
    shared: Arc<Mutex<SharedSample>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl MacosNetworkAttribution {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(SharedSample::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_shared = Arc::clone(&shared);
        let thread_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("batcave-macos-nstat".to_string())
            .spawn(move || run_collector(thread_shared, thread_stop));

        let worker = match worker {
            Ok(worker) => Some(worker),
            Err(error) => {
                set_failure(&shared, format!("nstat_thread_start_failed:{error}"));
                None
            }
        };

        Self {
            shared,
            stop,
            worker,
        }
    }

    pub fn sample(&mut self) -> NetworkAttributionSample {
        let Ok(mut shared) = self.shared.lock() else {
            return NetworkAttributionSample::Failed("nstat_sample_lock_poisoned".to_string());
        };
        if let Some(failure) = &shared.failure {
            return NetworkAttributionSample::Failed(failure.clone());
        }
        if !shared.baseline_complete {
            return NetworkAttributionSample::PendingBaseline(
                "Waiting for the initial macOS network-statistics baseline.".to_string(),
            );
        }

        let now = Instant::now();
        let elapsed = now.saturating_duration_since(shared.interval_started_at);
        if elapsed.is_zero() {
            return NetworkAttributionSample::Held(
                "Waiting for the next macOS network-statistics interval.".to_string(),
            );
        }
        shared.interval_started_at = now;
        let interval = std::mem::take(&mut shared.interval_bytes_by_process);
        let rates_by_process = interval
            .into_iter()
            .map(|(identity, bytes)| {
                (
                    identity,
                    ProcessNetworkRates {
                        received_bps: bytes_per_second(bytes.received_bps, elapsed),
                        transmitted_bps: bytes_per_second(bytes.transmitted_bps, elapsed),
                    },
                )
            })
            .collect();

        if let Some(message) = shared.data_loss.take() {
            NetworkAttributionSample::Partial {
                rates_by_process,
                message,
            }
        } else {
            NetworkAttributionSample::Ready { rates_by_process }
        }
    }
}

impl Default for MacosNetworkAttribution {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MacosNetworkAttribution {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_collector(shared: Arc<Mutex<SharedSample>>, stop: Arc<AtomicBool>) {
    if let Err(error) = run_collector_inner(&shared, &stop) {
        set_failure(&shared, error);
    }
}

fn run_collector_inner(shared: &Arc<Mutex<SharedSample>>, stop: &AtomicBool) -> Result<(), String> {
    ensure_qualified_darwin_layout()?;
    let socket = connect_nstat().map_err(|error| format!("nstat_connect_failed:{error}"))?;
    configure_socket(&socket).map_err(|error| format!("nstat_socket_config_failed:{error}"))?;

    let mut pending_subscriptions = PROVIDERS
        .into_iter()
        .map(|provider| (subscription_context(provider), provider))
        .collect::<HashMap<_, _>>();
    for provider in PROVIDERS {
        send_message(socket.as_raw_fd(), &add_all_request(provider))
            .map_err(|error| format!("nstat_subscribe_failed:provider={provider}:{error}"))?;
    }

    let mut engine = AttributionEngine::default();
    let mut receive_buffer = vec![0_u8; MAX_MESSAGE_SIZE];
    let mut query_context = 0x4e53_5400_0000_0001_u64;
    let mut active_query: Option<(u64, Instant, Vec<SourceUpdate>)> = None;
    let mut next_query_at = Instant::now();

    while !stop.load(Ordering::Acquire) {
        if pending_subscriptions.is_empty()
            && active_query.is_none()
            && Instant::now() >= next_query_at
        {
            send_message(socket.as_raw_fd(), &update_request(query_context, false))
                .map_err(|error| format!("nstat_update_request_failed:{error}"))?;
            active_query = Some((query_context, Instant::now(), Vec::new()));
            query_context = query_context.wrapping_add(1).max(0x4e53_5400_0000_0001);
        }

        if active_query
            .as_ref()
            .is_some_and(|(_, started, _)| started.elapsed() > QUERY_TIMEOUT)
        {
            return Err("nstat_update_request_timed_out".to_string());
        }

        let mut poll_fd = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll_fd points to one initialized pollfd for the duration of the call.
        let poll_result = unsafe { libc::poll(&mut poll_fd, 1, SOCKET_POLL_TIMEOUT_MS) };
        if poll_result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("nstat_poll_failed:{error}"));
        }
        if poll_result == 0 || poll_fd.revents & libc::POLLIN == 0 {
            if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Err(format!(
                    "nstat_poll_socket_error:revents={}",
                    poll_fd.revents
                ));
            }
            continue;
        }

        loop {
            // SAFETY: receive_buffer is valid writable storage and the socket is owned by this thread.
            let received = unsafe {
                libc::recv(
                    socket.as_raw_fd(),
                    receive_buffer.as_mut_ptr().cast(),
                    receive_buffer.len(),
                    0,
                )
            };
            if received < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    break;
                }
                return Err(format!("nstat_receive_failed:{error}"));
            }
            let received = received as usize;
            if received == receive_buffer.len() {
                return Err("nstat_message_truncated".to_string());
            }
            handle_datagram(
                &receive_buffer[..received],
                socket.as_raw_fd(),
                &mut pending_subscriptions,
                &mut active_query,
                &mut engine,
                shared,
                &mut next_query_at,
            )?;
        }
    }
    Ok(())
}

fn ensure_qualified_darwin_layout() -> Result<(), String> {
    // SAFETY: zero is a valid initial representation for utsname and uname fills it.
    let mut system: libc::utsname = unsafe { zeroed() };
    // SAFETY: system points to writable storage of the exact type uname expects.
    if unsafe { libc::uname(&mut system) } != 0 {
        return Err(format!(
            "nstat_darwin_version_unavailable:{}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: uname guarantees a NUL-terminated release field.
    let release = unsafe { CStr::from_ptr(system.release.as_ptr()) }
        .to_str()
        .map_err(|_| "nstat_darwin_version_invalid_utf8".to_string())?;
    let major = release
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| format!("nstat_darwin_version_invalid:{release}"))?;
    if !darwin_layout_is_qualified(major) {
        return Err(format!("nstat_darwin_layout_unqualified:{major}"));
    }
    Ok(())
}

fn darwin_layout_is_qualified(major: u32) -> bool {
    (MIN_QUALIFIED_DARWIN_MAJOR..=MAX_QUALIFIED_DARWIN_MAJOR).contains(&major)
}

#[allow(clippy::too_many_arguments)]
fn handle_datagram(
    datagram: &[u8],
    socket: RawFd,
    pending_subscriptions: &mut HashMap<u64, u32>,
    active_query: &mut Option<(u64, Instant, Vec<SourceUpdate>)>,
    engine: &mut AttributionEngine,
    shared: &Arc<Mutex<SharedSample>>,
    next_query_at: &mut Instant,
) -> Result<(), String> {
    let mut offset = 0;
    while offset < datagram.len() {
        let header = parse_header(datagram.get(offset..).unwrap_or_default())?;
        let end = offset
            .checked_add(header.length)
            .filter(|end| *end <= datagram.len())
            .ok_or_else(|| "nstat_message_length_out_of_bounds".to_string())?;
        let message = &datagram[offset..end];

        match header.message_type {
            NSTAT_MSG_TYPE_SUCCESS => {
                if pending_subscriptions.remove(&header.context).is_some() {
                    if pending_subscriptions.is_empty() {
                        *next_query_at = Instant::now();
                    }
                } else if active_query
                    .as_ref()
                    .is_some_and(|(context, _, _)| *context == header.context)
                {
                    if header.flags & NSTAT_MSG_HDR_FLAG_CONTINUATION != 0 {
                        send_message(socket, &update_request(header.context, true))
                            .map_err(|error| format!("nstat_update_continuation_failed:{error}"))?;
                    } else {
                        let (_, _, updates) = active_query
                            .take()
                            .expect("active query checked before completion");
                        commit_updates(engine, updates, shared)?;
                        if !engine.baseline_complete {
                            engine.finish_baseline();
                            let mut sample = shared
                                .lock()
                                .map_err(|_| "nstat_shared_lock_poisoned".to_string())?;
                            sample.baseline_complete = true;
                            sample.interval_bytes_by_process.clear();
                            sample.interval_started_at = Instant::now();
                        }
                        *next_query_at = Instant::now() + POLL_INTERVAL;
                    }
                }
            }
            NSTAT_MSG_TYPE_ERROR => {
                let error = read_u32(message, HEADER_LEN)?;
                if let Some(provider) = pending_subscriptions.remove(&header.context) {
                    return Err(format!(
                        "nstat_subscription_rejected:provider={provider}:errno={error}"
                    ));
                }
                if active_query
                    .as_ref()
                    .is_some_and(|(context, _, _)| *context == header.context)
                {
                    return Err(format!("nstat_update_rejected:errno={error}"));
                }
            }
            NSTAT_MSG_TYPE_SRC_UPDATE | NSTAT_MSG_TYPE_SRC_EXTENDED_UPDATE => {
                let update = parse_source_update(message, header)?;
                if header.context == 0 {
                    commit_updates(engine, vec![update], shared)?;
                } else if let Some((context, _, updates)) = active_query.as_mut() {
                    if *context != header.context {
                        return Err(format!(
                            "nstat_update_context_mismatch:expected={context}:actual={}",
                            header.context
                        ));
                    }
                    updates.push(update);
                }
            }
            NSTAT_MSG_TYPE_SRC_REMOVED => {
                let source_ref = read_u64(message, HEADER_LEN)?;
                engine.remove_source(source_ref);
            }
            NSTAT_MSG_TYPE_SRC_ADDED => {}
            _ => {}
        }
        offset = end;
    }
    Ok(())
}

fn commit_updates(
    engine: &mut AttributionEngine,
    updates: Vec<SourceUpdate>,
    shared: &Arc<Mutex<SharedSample>>,
) -> Result<(), String> {
    let mut sample = shared
        .lock()
        .map_err(|_| "nstat_shared_lock_poisoned".to_string())?;
    for update in updates {
        if let Some(message) = engine.apply_update(update, &mut sample.interval_bytes_by_process) {
            sample.data_loss = Some(message);
        }
    }
    Ok(())
}

fn connect_nstat() -> io::Result<OwnedFd> {
    // SAFETY: socket returns a new descriptor or -1 and has no pointer arguments.
    let raw = unsafe { libc::socket(libc::PF_SYSTEM, libc::SOCK_DGRAM, libc::SYSPROTO_CONTROL) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw was returned as a new owned descriptor above.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: zero is a valid initial representation for ctl_info.
    let mut info: libc::ctl_info = unsafe { zeroed() };
    for (target, source) in info.ctl_name.iter_mut().zip(CONTROL_NAME.iter().copied()) {
        *target = source as libc::c_char;
    }
    // SAFETY: info is writable storage of the exact type required by CTLIOCGINFO.
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::CTLIOCGINFO, &mut info) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: zero is a valid initial representation for sockaddr_ctl.
    let mut address: libc::sockaddr_ctl = unsafe { zeroed() };
    address.sc_len = size_of::<libc::sockaddr_ctl>() as u8;
    address.sc_family = libc::AF_SYSTEM as u8;
    address.ss_sysaddr = libc::AF_SYS_CONTROL as u16;
    address.sc_id = info.ctl_id;
    // SAFETY: address is a fully initialized sockaddr_ctl and the length matches it.
    if unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&raw const address).cast(),
            size_of::<libc::sockaddr_ctl>() as libc::socklen_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

fn configure_socket(socket: &OwnedFd) -> io::Result<()> {
    // SAFETY: fcntl operates on the valid owned descriptor and returns flags or -1.
    let flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let receive_buffer_size = RECEIVE_BUFFER_SIZE;
    // SAFETY: the value pointer and length describe receive_buffer_size exactly.
    if unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&raw const receive_buffer_size).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn send_message(socket: RawFd, message: &[u8]) -> io::Result<()> {
    // SAFETY: message is a readable byte slice and socket is valid for the collector lifetime.
    let sent = unsafe { libc::send(socket, message.as_ptr().cast(), message.len(), 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    if sent as usize != message.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short NStat datagram write",
        ));
    }
    Ok(())
}

fn add_all_request(provider: u32) -> Vec<u8> {
    let mut request = Vec::with_capacity(56);
    write_header(
        &mut request,
        subscription_context(provider),
        NSTAT_MSG_TYPE_ADD_ALL_SRCS,
        56,
        0,
    );
    request.extend_from_slice(
        &(NSTAT_FILTER_PROVIDER_NOZEROBYTES | NSTAT_FILTER_PROVIDER_NOZERODELTAS).to_le_bytes(),
    );
    request.extend_from_slice(&0_u64.to_le_bytes());
    request.extend_from_slice(&provider.to_le_bytes());
    request.extend_from_slice(&0_i32.to_le_bytes());
    request.extend_from_slice(&[0_u8; 16]);
    request
}

fn update_request(context: u64, continuation: bool) -> Vec<u8> {
    let mut request = Vec::with_capacity(24);
    write_header(
        &mut request,
        context,
        NSTAT_MSG_TYPE_GET_UPDATE,
        24,
        if continuation {
            NSTAT_MSG_HDR_FLAG_CONTINUATION
        } else {
            0
        },
    );
    request.extend_from_slice(&NSTAT_SRC_REF_ALL.to_le_bytes());
    request
}

fn write_header(target: &mut Vec<u8>, context: u64, message_type: u32, length: u16, flags: u16) {
    target.extend_from_slice(&context.to_le_bytes());
    target.extend_from_slice(&message_type.to_le_bytes());
    target.extend_from_slice(&length.to_le_bytes());
    target.extend_from_slice(&flags.to_le_bytes());
}

fn subscription_context(provider: u32) -> u64 {
    0x4e53_5300_0000_0000 | u64::from(provider)
}

fn parse_header(bytes: &[u8]) -> Result<MessageHeader, String> {
    if bytes.len() < HEADER_LEN {
        return Err("nstat_message_header_truncated".to_string());
    }
    let length = usize::from(read_u16(bytes, 12)?);
    if length < HEADER_LEN {
        return Err(format!("nstat_message_length_invalid:{length}"));
    }
    Ok(MessageHeader {
        context: read_u64(bytes, 0)?,
        message_type: read_u32(bytes, 8)?,
        length,
        flags: read_u16(bytes, 14)?,
    })
}

fn parse_source_update(bytes: &[u8], header: MessageHeader) -> Result<SourceUpdate, String> {
    if bytes.len() < UPDATE_PREFIX_LEN {
        return Err("nstat_source_update_truncated".to_string());
    }
    let provider = read_u32(bytes, UPDATE_PROVIDER_OFFSET)?;
    let descriptor_pid_offset = match provider {
        NSTAT_PROVIDER_TCP_KERNEL | NSTAT_PROVIDER_TCP_USERLAND | NSTAT_PROVIDER_QUIC_USERLAND => {
            TCP_DESCRIPTOR_PID_OFFSET
        }
        NSTAT_PROVIDER_UDP_KERNEL | NSTAT_PROVIDER_UDP_USERLAND => UDP_DESCRIPTOR_PID_OFFSET,
        _ => return Err(format!("nstat_provider_unexpected:{provider}")),
    };
    let pid = read_u32(bytes, UPDATE_PREFIX_LEN + descriptor_pid_offset)?;
    let unique_pid = read_u64(bytes, UPDATE_PREFIX_LEN)?;
    if pid == 0 {
        return Err(format!("nstat_source_pid_missing:provider={provider}"));
    }
    Ok(SourceUpdate {
        source_ref: read_u64(bytes, UPDATE_SOURCE_REF_OFFSET)?,
        pid,
        unique_pid,
        received_bytes: read_u64(bytes, UPDATE_RX_BYTES_OFFSET)?,
        transmitted_bytes: read_u64(bytes, UPDATE_TX_BYTES_OFFSET)?,
        closing: header.flags & NSTAT_MSG_HDR_FLAG_CLOSING != 0,
        previous_event_discarded: read_u64(bytes, HEADER_LEN + size_of::<u64>())?
            & NSTAT_EVENT_SRC_PREV_EVENT_DISCARDED
            != 0,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| format!("nstat_u16_out_of_bounds:offset={offset}"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("nstat_u32_out_of_bounds:offset={offset}"))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    bytes
        .get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| format!("nstat_u64_out_of_bounds:offset={offset}"))
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    let nanos = elapsed.as_nanos().max(1);
    (u128::from(bytes)
        .saturating_mul(1_000_000_000)
        .checked_div(nanos)
        .unwrap_or_default())
    .min(u128::from(u64::MAX)) as u64
}

fn set_failure(shared: &Arc<Mutex<SharedSample>>, message: String) {
    if let Ok(mut shared) = shared.lock() {
        shared.failure = Some(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
    };

    fn update(source_ref: u64, pid: u32, received: u64, transmitted: u64) -> SourceUpdate {
        SourceUpdate {
            source_ref,
            pid,
            unique_pid: u64::from(pid),
            received_bytes: received,
            transmitted_bytes: transmitted,
            closing: false,
            previous_event_discarded: false,
        }
    }

    #[test]
    fn wire_requests_match_the_xnu_layout() {
        let add = add_all_request(NSTAT_PROVIDER_TCP_KERNEL);
        assert_eq!(add.len(), 56);
        assert_eq!(read_u32(&add, 8).unwrap(), NSTAT_MSG_TYPE_ADD_ALL_SRCS);
        assert_eq!(read_u16(&add, 12).unwrap(), 56);
        assert_eq!(read_u32(&add, 32).unwrap(), NSTAT_PROVIDER_TCP_KERNEL);

        let query = update_request(77, true);
        assert_eq!(query.len(), 24);
        assert_eq!(read_u64(&query, 0).unwrap(), 77);
        assert_eq!(read_u32(&query, 8).unwrap(), NSTAT_MSG_TYPE_GET_UPDATE);
        assert_eq!(
            read_u16(&query, 14).unwrap(),
            NSTAT_MSG_HDR_FLAG_CONTINUATION
        );
        assert_eq!(read_u64(&query, 16).unwrap(), NSTAT_SRC_REF_ALL);
    }

    #[test]
    fn only_source_qualified_darwin_layouts_are_enabled() {
        assert!(!darwin_layout_is_qualified(20));
        assert!(darwin_layout_is_qualified(21));
        assert!(darwin_layout_is_qualified(25));
        assert!(!darwin_layout_is_qualified(26));
    }

    #[test]
    fn baseline_does_not_publish_historical_bytes() {
        let mut engine = AttributionEngine::default();
        let mut interval = HashMap::new();
        engine.apply_update(update(1, 42, 1_000, 2_000), &mut interval);
        assert!(interval.is_empty());

        engine.finish_baseline();
        engine.apply_update(update(1, 42, 1_400, 2_700), &mut interval);
        assert_eq!(
            interval.get(&ObservedProcessGeneration::platform(42, 42)),
            Some(&ProcessNetworkRates {
                received_bps: 400,
                transmitted_bps: 700,
            })
        );
    }

    #[test]
    fn new_and_closing_sources_preserve_short_lived_flow_bytes() {
        let mut engine = AttributionEngine::default();
        engine.finish_baseline();
        let mut interval = HashMap::new();
        let mut closing = update(2, 51, 900, 300);
        closing.closing = true;

        engine.apply_update(closing, &mut interval);

        assert_eq!(
            interval.get(&ObservedProcessGeneration::platform(51, 51)),
            Some(&ProcessNetworkRates {
                received_bps: 900,
                transmitted_bps: 300,
            })
        );
        assert!(!engine.sources.contains_key(&2));
    }

    #[test]
    fn source_generation_change_does_not_bill_either_pid() {
        let mut engine = AttributionEngine::default();
        let mut interval = HashMap::new();
        engine.apply_update(update(3, 60, 100, 200), &mut interval);
        engine.finish_baseline();

        let loss = engine.apply_update(update(3, 61, 150, 260), &mut interval);

        assert!(loss
            .as_deref()
            .is_some_and(|message| message.contains("source_generation_changed")));
        assert!(interval.is_empty());
    }

    #[test]
    fn counter_regression_is_reported_without_wrapping() {
        let mut engine = AttributionEngine::default();
        let mut interval = HashMap::new();
        engine.apply_update(update(4, 70, 100, 200), &mut interval);
        engine.finish_baseline();

        let loss = engine.apply_update(update(4, 70, 90, 210), &mut interval);

        assert_eq!(
            loss.as_deref(),
            Some("nstat_counter_regressed:source_ref=4:unique_pid=70")
        );
        assert!(interval.is_empty());
    }

    #[test]
    fn discarded_kernel_event_marks_the_interval_partial() {
        let mut engine = AttributionEngine::default();
        engine.finish_baseline();
        let mut interval = HashMap::new();
        let mut discarded = update(5, 71, 100, 200);
        discarded.previous_event_discarded = true;

        let loss = engine.apply_update(discarded, &mut interval);

        assert_eq!(
            loss.as_deref(),
            Some("nstat_previous_event_discarded:source_ref=5")
        );
    }

    #[test]
    fn source_update_parser_rejects_unknown_or_truncated_layouts() {
        let header = MessageHeader {
            context: 1,
            message_type: NSTAT_MSG_TYPE_SRC_UPDATE,
            length: UPDATE_PREFIX_LEN,
            flags: 0,
        };
        assert_eq!(
            parse_source_update(&[0; UPDATE_PREFIX_LEN], header).unwrap_err(),
            "nstat_provider_unexpected:0"
        );
    }

    #[test]
    fn native_control_socket_reaches_a_complete_baseline() {
        let mut attribution = MacosNetworkAttribution::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match attribution.sample() {
                NetworkAttributionSample::Ready { .. }
                | NetworkAttributionSample::Partial { .. } => break,
                NetworkAttributionSample::PendingBaseline(_)
                | NetworkAttributionSample::Held(_)
                    if Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(50));
                }
                sample => panic!("NStat did not reach a usable baseline: {sample:?}"),
            }
        }
    }

    #[test]
    fn native_control_socket_attributes_loopback_bytes_to_the_process() {
        let mut attribution = MacosNetworkAttribution::new();
        wait_for_baseline(&mut attribution);

        const PAYLOAD_BYTES: usize = 512 * 1024;
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
        let address = listener.local_addr().expect("loopback address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept loopback client");
            let mut received = vec![0_u8; PAYLOAD_BYTES];
            stream.read_exact(&mut received).expect("read upload");
            stream.write_all(&received).expect("write download");
            stream.shutdown(Shutdown::Write).expect("finish download");
        });

        let mut client = TcpStream::connect(address).expect("connect loopback client");
        client
            .write_all(&vec![0x5a; PAYLOAD_BYTES])
            .expect("write upload");
        client.shutdown(Shutdown::Write).expect("finish upload");
        let mut response = Vec::with_capacity(PAYLOAD_BYTES);
        client.read_to_end(&mut response).expect("read download");
        assert_eq!(response.len(), PAYLOAD_BYTES);
        server.join().expect("loopback server");

        let pid = std::process::id();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            thread::sleep(Duration::from_millis(100));
            match attribution.sample() {
                NetworkAttributionSample::Ready { rates_by_process }
                | NetworkAttributionSample::Partial {
                    rates_by_process, ..
                } if rates_by_process.iter().any(|(identity, rates)| {
                    identity.pid == pid && rates.received_bps > 0 && rates.transmitted_bps > 0
                }) =>
                {
                    break;
                }
                NetworkAttributionSample::Failed(message) => {
                    panic!("NStat failed while attributing loopback traffic: {message}")
                }
                _ if Instant::now() < deadline => {}
                sample => panic!("NStat did not attribute loopback traffic: {sample:?}"),
            }
        }
    }

    fn wait_for_baseline(attribution: &mut MacosNetworkAttribution) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match attribution.sample() {
                NetworkAttributionSample::Ready { .. }
                | NetworkAttributionSample::Partial { .. } => break,
                NetworkAttributionSample::PendingBaseline(_)
                | NetworkAttributionSample::Held(_)
                    if Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(50));
                }
                sample => panic!("NStat did not reach a usable baseline: {sample:?}"),
            }
        }
    }
}
