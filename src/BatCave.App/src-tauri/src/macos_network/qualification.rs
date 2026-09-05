use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
};

use super::*;

#[cfg(test)]
mod tests;

const QUALIFICATION_TIMEOUT: Duration = Duration::from_secs(6);
const PROBE_IO_TIMEOUT: Duration = Duration::from_millis(250);

// Qualify the fields we consume, rather than an OS release number. The probe
// stays on loopback, uses distinct byte counts in each direction, and checks
// NStat's identity against libproc. Descriptor tails may grow without changing
// those fields. Each new control-socket session must pass independently.
pub(super) struct ProtocolQualification {
    started_at: Instant,
    unique_pid: u64,
    flows: Vec<ExpectedFlow>,
    sockets: Option<(TcpStream, TcpStream, UdpSocket, UdpSocket)>,
}

struct ExpectedFlow {
    tcp: bool,
    local_port: u16,
    remote_port: u16,
    received: u64,
    transmitted: u64,
    source_ref: Option<u64>,
}

impl ProtocolQualification {
    #[cfg(test)]
    pub(super) fn qualified_fixture() -> Self {
        Self {
            started_at: Instant::now(),
            unique_pid: 1,
            flows: Vec::new(),
            sockets: None,
        }
    }

    pub(super) fn new() -> Result<Self, String> {
        Self::open().map_err(|error| format!("nstat_protocol_probe_failed:{error}"))
    }

    fn open() -> io::Result<Self> {
        let unique_pid = own_unique_pid()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let mut client = TcpStream::connect_timeout(&listener.local_addr()?, PROBE_IO_TIMEOUT)?;
        let mut poll_fd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll_fd is one initialized pollfd, owned for this call. The
        // accept queue can lag connect completion on macOS; keep the wait bounded.
        if unsafe { libc::poll(&mut poll_fd, 1, PROBE_IO_TIMEOUT.as_millis() as i32) } != 1 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loopback accept timed out",
            ));
        }
        let (mut server, _) = listener.accept()?;
        for stream in [&client, &server] {
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(PROBE_IO_TIMEOUT))?;
            stream.set_write_timeout(Some(PROBE_IO_TIMEOUT))?;
        }
        let udp_client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        let udp_server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        udp_client.connect(udp_server.local_addr()?)?;
        udp_server.connect(udp_client.local_addr()?)?;
        for socket in [&udp_client, &udp_server] {
            socket.set_read_timeout(Some(PROBE_IO_TIMEOUT))?;
            socket.set_write_timeout(Some(PROBE_IO_TIMEOUT))?;
        }

        client.write_all(&[0x53; 4093])?;
        server.read_exact(&mut [0; 4093])?;
        server.write_all(&[0x71; 2053])?;
        client.read_exact(&mut [0; 2053])?;
        udp_client.send(&[0x37; 509])?;
        if udp_server.recv(&mut [0; 1024])? != 509 {
            return Err(io::Error::other("UDP upload length mismatch"));
        }
        udp_server.send(&[0x19; 257])?;
        if udp_client.recv(&mut [0; 1024])? != 257 {
            return Err(io::Error::other("UDP download length mismatch"));
        }

        let flows = vec![
            ExpectedFlow::new(true, client.local_addr()?, client.peer_addr()?, 2053, 4093),
            ExpectedFlow::new(true, server.local_addr()?, server.peer_addr()?, 4093, 2053),
            ExpectedFlow::new(
                false,
                udp_client.local_addr()?,
                udp_client.peer_addr()?,
                257,
                509,
            ),
            ExpectedFlow::new(
                false,
                udp_server.local_addr()?,
                udp_server.peer_addr()?,
                509,
                257,
            ),
        ];
        Ok(Self {
            started_at: Instant::now(),
            unique_pid,
            flows,
            sockets: Some((client, server, udp_client, udp_server)),
        })
    }

    pub(super) fn observe(&mut self, bytes: &[u8], update: SourceUpdate) -> Result<(), String> {
        if self.complete() {
            return Ok(());
        }
        let provider = read_u32(bytes, UPDATE_PROVIDER_OFFSET)?;
        let tcp = matches!(
            provider,
            NSTAT_PROVIDER_TCP_KERNEL | NSTAT_PROVIDER_TCP_USERLAND | NSTAT_PROVIDER_QUIC_USERLAND
        );
        let local_offset = UPDATE_PREFIX_LEN + if tcp { 124 } else { 56 };
        let local_port = loopback_port(bytes, local_offset);
        let remote_port = loopback_port(bytes, local_offset + 28);
        if let Some(index) = self.flows.iter().position(|flow| {
            flow.tcp == tcp
                && local_port == Some(flow.local_port)
                && remote_port == Some(flow.remote_port)
        }) {
            if self
                .flows
                .iter()
                .enumerate()
                .any(|(other, flow)| other != index && flow.source_ref == Some(update.source_ref))
            {
                return Err("nstat_protocol_probe_source_collision".to_string());
            }
            let flow = &mut self.flows[index];
            if update.pid != std::process::id()
                || update.unique_pid != self.unique_pid
                || update.received_bytes != flow.received
                || update.transmitted_bytes != flow.transmitted
                || update.previous_event_discarded
                || update.closing
                || flow
                    .source_ref
                    .is_some_and(|source| source != update.source_ref)
            {
                return Err(format!("nstat_protocol_probe_mismatch:provider={provider}"));
            }
            flow.source_ref = Some(update.source_ref);
        }
        Ok(())
    }

    pub(super) fn complete(&self) -> bool {
        self.flows.iter().all(|flow| flow.source_ref.is_some())
    }

    pub(super) fn excludes(&self, source_ref: u64) -> bool {
        self.flows
            .iter()
            .any(|flow| flow.source_ref == Some(source_ref))
    }

    pub(super) fn finish(&mut self) {
        // Retain the source references so final close updates cannot bill the
        // qualification traffic to BatCave after the baseline has completed.
        self.sockets.take();
    }

    pub(super) fn check_deadline(&self) -> Result<(), String> {
        if !self.complete() && self.started_at.elapsed() > QUALIFICATION_TIMEOUT {
            return Err("nstat_protocol_probe_timed_out".to_string());
        }
        Ok(())
    }
}

impl ExpectedFlow {
    fn new(
        tcp: bool,
        local: SocketAddr,
        remote: SocketAddr,
        received: u64,
        transmitted: u64,
    ) -> Self {
        Self {
            tcp,
            local_port: local.port(),
            remote_port: remote.port(),
            received,
            transmitted,
            source_ref: None,
        }
    }
}

fn loopback_port(bytes: &[u8], offset: usize) -> Option<u16> {
    let address = bytes.get(offset..offset + 16)?;
    (address[0] == 16 && address[1] == libc::AF_INET as u8 && address[4..8] == [127, 0, 0, 1])
        .then(|| u16::from_be_bytes([address[2], address[3]]))
}

fn own_unique_pid() -> io::Result<u64> {
    // Apple's proc_info_private.h declares this 56-byte ABI explicitly stable:
    // proc_uniqidentifierinfo, PROC_PIDUNIQIDENTIFIERINFO (17), unique ID at 16.
    // https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/proc_info_private.h
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: libc::c_int,
            flavor: libc::c_int,
            arg: u64,
            buffer: *mut libc::c_void,
            size: libc::c_int,
        ) -> libc::c_int;
    }
    let mut info = [0_u8; 56];
    // SAFETY: info is writable storage of the specified ABI size; this queries
    // only the current process, which remains alive for the entire call.
    let received = unsafe {
        proc_pidinfo(
            std::process::id() as i32,
            17,
            0,
            info.as_mut_ptr().cast(),
            info.len() as i32,
        )
    };
    if received != info.len() as i32 {
        return Err(io::Error::other(format!(
            "own process identity read returned {received}: {}",
            io::Error::last_os_error()
        )));
    }
    let unique_pid = u64::from_ne_bytes(info[16..24].try_into().expect("fixed identity field"));
    if unique_pid == 0 {
        return Err(io::Error::other("own process unique identity missing"));
    }
    Ok(unique_pid)
}
