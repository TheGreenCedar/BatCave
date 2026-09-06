use super::*;
use std::os::unix::net::UnixDatagram;

// These fixture offsets and literal flags come from the published wire layout,
// independently of the parser constants. No Apple control socket is opened.
pub(super) fn message(kind: u32, context: u64, flags: u16, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    bytes[0..8].copy_from_slice(&context.to_le_bytes());
    bytes[8..12].copy_from_slice(&kind.to_le_bytes());
    bytes[12..14].copy_from_slice(&(length as u16).to_le_bytes());
    bytes[14..16].copy_from_slice(&flags.to_le_bytes());
    bytes
}

pub(super) fn source_update(provider: u32, tail: usize) -> Vec<u8> {
    let tcp = matches!(provider, 2 | 3 | 8);
    let mut bytes = message(10_006, 0, 0, 152 + if tcp { 344 } else { 280 } + tail);
    bytes[16..24].copy_from_slice(&17_u64.to_le_bytes());
    bytes[40..48].copy_from_slice(&123_u64.to_le_bytes());
    bytes[56..64].copy_from_slice(&456_u64.to_le_bytes());
    bytes[144..148].copy_from_slice(&provider.to_le_bytes());
    bytes[152..160].copy_from_slice(&9001_u64.to_le_bytes());
    let pid_offset = 152 + if tcp { 116 } else { 128 };
    bytes[pid_offset..pid_offset + 4].copy_from_slice(&42_u32.to_le_bytes());
    bytes
}

fn decode(bytes: &[u8]) -> Result<SourceUpdate, String> {
    let header = parse_header(bytes)?;
    validate_message_shape(bytes, header)?;
    parse_source_update(bytes, header)
}

pub(super) struct WireHarness {
    pub(super) pending: HashMap<u64, u32>,
    pub(super) active: Option<ActiveQuery>,
    pub(super) engine: AttributionEngine,
    clock: EventClock,
    pub(super) shared: Arc<Mutex<SharedSample>>,
    next_query: Instant,
    pub(super) qualification: ProtocolQualification,
}

impl WireHarness {
    pub(super) fn new(qualification: ProtocolQualification) -> Self {
        Self {
            pending: HashMap::new(),
            active: None,
            engine: AttributionEngine::default(),
            clock: EventClock::default(),
            shared: Arc::new(Mutex::new(SharedSample::default())),
            next_query: Instant::now(),
            qualification,
        }
    }

    pub(super) fn query(&mut self, context: u64) {
        assert!(self.active.is_none());
        self.active = Some(ActiveQuery::new(
            context,
            self.clock.next_epoch(),
            Instant::now(),
        ));
    }

    pub(super) fn feed(&mut self, bytes: &[u8], socket: RawFd) -> Result<(), String> {
        handle_datagram(
            bytes,
            socket,
            &mut self.pending,
            &mut self.active,
            &mut self.engine,
            &mut self.clock,
            &self.shared,
            &mut self.next_query,
            &mut self.qualification,
        )
    }

    pub(super) fn sample(&self) -> NetworkAttributionSample {
        MacosNetworkAttribution {
            shared: Arc::clone(&self.shared),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
        .sample()
    }
}

#[test]
fn literal_continuation_requests_same_context_without_completing_either_interval() {
    let (client, peer) = UnixDatagram::pair().expect("local datagram pair");
    peer.set_nonblocking(true).unwrap();
    let mut harness = WireHarness::new(ProtocolQualification::qualified_fixture());
    let mut bytes = source_update(2, 0);
    for (context, received, baseline_complete) in [(81_u64, 123_u64, false), (82, 200, true)] {
        harness.query(context);
        bytes[0..8].copy_from_slice(&context.to_le_bytes());
        bytes[40..48].copy_from_slice(&received.to_le_bytes());
        // Multiple logical messages in one datagram exercise aggregate parsing.
        let mut fragment = bytes.clone();
        fragment.extend(message(0, context, 0x0002, 16));
        harness.feed(&fragment, client.as_raw_fd()).unwrap();

        assert_eq!(harness.active.as_ref().unwrap().context, context);
        assert_eq!(harness.active.as_ref().unwrap().events.len(), 1);
        let shared = harness.shared.lock().unwrap();
        assert_eq!(shared.baseline_complete, baseline_complete);
        assert!(!shared.interval_complete);
        assert!(shared.interval_bytes_by_process.is_empty());
        drop(shared);
        assert!(matches!(
            harness.sample(),
            NetworkAttributionSample::PendingBaseline(_) | NetworkAttributionSample::Held(_)
        ));

        let mut request = [0_u8; 64];
        assert_eq!(peer.recv(&mut request).unwrap(), 24);
        assert_eq!(&request[0..8], &context.to_le_bytes());
        assert_eq!(&request[8..12], &1007_u32.to_le_bytes());
        assert_eq!(&request[12..16], &[24, 0, 2, 0]);
        assert_eq!(&request[16..24], &u64::MAX.to_le_bytes());

        harness
            .feed(&message(0, context, 0, 16), client.as_raw_fd())
            .unwrap();
        assert!(harness.active.is_none());
        assert_eq!(
            peer.recv(&mut request).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    let shared = harness.shared.lock().unwrap();
    assert!(shared.interval_complete);
    let bytes = shared
        .interval_bytes_by_process
        .get(&ObservedProcessGeneration::platform(42, 9001))
        .unwrap();
    assert_eq!(bytes.received_bps, 77);
    assert_eq!(bytes.transmitted_bps, 0);
}

#[test]
fn literal_dropped_final_counts_removal_marks_partial_but_other_close_reasons_do_not() {
    for flags in [0x0000_u16, 0x0008, 0x0010, 0x0040] {
        let mut harness = WireHarness::new(ProtocolQualification::qualified_fixture());
        harness.feed(&source_update(2, 0), -1).unwrap();
        harness.query(91);
        harness.feed(&message(0, 91, 0, 16), -1).unwrap();
        let mut removal = message(10_002, 0, flags, 24);
        removal[16..24].copy_from_slice(&17_u64.to_le_bytes());
        harness.feed(&removal, -1).unwrap();
        assert!(!harness.engine.sources.contains_key(&17));
        harness.query(92);
        harness.feed(&message(0, 92, 0, 16), -1).unwrap();
        let sample = harness.sample();
        if flags == 0x0008 {
            assert!(
                matches!(sample, NetworkAttributionSample::Partial { message, .. }
                if message == "nstat_final_counts_dropped:source_ref=17")
            );
        } else {
            assert!(matches!(sample, NetworkAttributionSample::Ready { .. }));
        }
    }
}

#[test]
fn all_five_provider_prefixes_accept_only_complete_aligned_layouts() {
    for provider in [2_u32, 3, 4, 5, 8] {
        for tail in [0, 8, 16] {
            let bytes = source_update(provider, tail);
            let parsed = decode(&bytes).unwrap();
            assert_eq!(
                (parsed.source_ref, parsed.pid, parsed.unique_pid),
                (17, 42, 9001)
            );
            assert_eq!(
                (parsed.received_bytes, parsed.transmitted_bytes),
                (123, 456)
            );
        }
        for delta in [-8_isize, -1, 1, 7] {
            let mut bytes = source_update(provider, 0);
            bytes.resize(bytes.len().checked_add_signed(delta).unwrap(), 0);
            let length = bytes.len() as u16;
            bytes[12..14].copy_from_slice(&length.to_le_bytes());
            assert!(
                decode(&bytes).is_err(),
                "provider={provider}, delta={delta}"
            );
        }
        for offset in [
            148,
            152,
            152 + if matches!(provider, 2 | 3 | 8) {
                116
            } else {
                128
            },
        ] {
            let mut bytes = source_update(provider, 0);
            if offset == 148 {
                bytes[offset] = 1;
            } else {
                let width = if offset == 152 { 8 } else { 4 };
                bytes[offset..offset + width].fill(0);
            }
            assert!(
                decode(&bytes).is_err(),
                "provider={provider}, offset={offset}"
            );
        }
        for source_ref in [0_u64, u64::MAX] {
            let mut bytes = source_update(provider, 0);
            bytes[16..24].copy_from_slice(&source_ref.to_le_bytes());
            assert!(decode(&bytes).is_err());
        }
    }
}

#[test]
fn unknown_source_messages_extensions_and_malformed_envelopes_fail_before_completion() {
    let mut cases = Vec::new();
    for kind in [10_003_u32, 10_007, 10_008, u32::MAX] {
        let mut bytes = source_update(2, 16);
        bytes[8..12].copy_from_slice(&kind.to_le_bytes());
        cases.push(bytes);
    }
    let mut unknown_provider = source_update(2, 0);
    unknown_provider[144..148].copy_from_slice(&9_u32.to_le_bytes());
    cases.push(unknown_provider);
    cases.push(message(0, 101, 0, 24));
    cases.push(message(0, 101, 0x0020, 16));
    cases.push(message(10_002, 0, 0, 32));
    let mut added = message(10_001, 0, 0, 32);
    added[24..28].copy_from_slice(&2_u32.to_le_bytes());
    added[28] = 1;
    cases.push(added);
    let mut error = message(1, 101, 0, 24);
    error[20] = 1;
    cases.push(error);
    for mut bytes in cases {
        let mut harness = WireHarness::new(ProtocolQualification::qualified_fixture());
        harness.query(101);
        bytes.extend(message(0, 101, 0, 16));
        assert!(harness.feed(&bytes, -1).is_err());
        assert!(harness.active.is_some());
        assert!(!harness.shared.lock().unwrap().baseline_complete);
    }
}

#[test]
fn subscription_counts_are_baseline_history_and_cannot_be_accepted_as_live_updates() {
    let mut harness = WireHarness::new(ProtocolQualification::qualified_fixture());
    let mut counts = message(10_004, 0, 0x0004, 144);
    counts[16..24].copy_from_slice(&27_u64.to_le_bytes());
    counts[40..48].copy_from_slice(&999_u64.to_le_bytes());
    harness.feed(&counts, -1).unwrap();
    assert!(!harness.engine.baseline_complete);
    assert!(harness.engine.sources.is_empty());
    assert!(harness
        .shared
        .lock()
        .unwrap()
        .interval_bytes_by_process
        .is_empty());
    harness.query(201);
    harness.feed(&message(0, 201, 0, 16), -1).unwrap();
    assert!(harness.engine.baseline_complete);
    assert!(harness.feed(&counts, -1).is_err());
    assert!(harness
        .shared
        .lock()
        .unwrap()
        .interval_bytes_by_process
        .is_empty());
}
