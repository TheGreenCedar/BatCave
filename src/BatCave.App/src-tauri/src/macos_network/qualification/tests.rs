use super::*;
use crate::macos_network::wire_tests::{message, source_update, WireHarness};

const OWN_UNIQUE_PID: u64 = 7007;
const FLOWS: [(bool, u16, u16, u64, u64); 4] = [
    (true, 30001, 30002, 2053, 4093),
    (true, 30002, 30001, 4093, 2053),
    (false, 31001, 31002, 257, 509),
    (false, 31002, 31001, 509, 257),
];

fn pending_qualification() -> ProtocolQualification {
    ProtocolQualification {
        started_at: Instant::now(),
        unique_pid: OWN_UNIQUE_PID,
        flows: FLOWS
            .into_iter()
            .map(
                |(tcp, local_port, remote_port, received, transmitted)| ExpectedFlow {
                    tcp,
                    local_port,
                    remote_port,
                    received,
                    transmitted,
                    source_ref: None,
                },
            )
            .collect(),
        sockets: None,
    }
}

fn probe_message(index: usize) -> Vec<u8> {
    let (tcp, local, remote, received, transmitted) = FLOWS[index];
    let mut bytes = source_update(if tcp { 2 } else { 4 }, if tcp { 16 } else { 0 });
    bytes[16..24].copy_from_slice(&(index as u64 + 101).to_le_bytes());
    bytes[40..48].copy_from_slice(&received.to_le_bytes());
    bytes[56..64].copy_from_slice(&transmitted.to_le_bytes());
    bytes[152..160].copy_from_slice(&OWN_UNIQUE_PID.to_le_bytes());
    let pid_offset = 152 + if tcp { 116 } else { 128 };
    bytes[pid_offset..pid_offset + 4].copy_from_slice(&std::process::id().to_le_bytes());
    let local_offset = 152 + if tcp { 124 } else { 56 };
    for (offset, port) in [(local_offset, local), (local_offset + 28, remote)] {
        bytes[offset] = 16;
        bytes[offset + 1] = 2; // Darwin AF_INET, independent of the parser.
        bytes[offset + 2..offset + 4].copy_from_slice(&port.to_be_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&[127, 0, 0, 1]);
    }
    bytes
}

fn observe(qualification: &mut ProtocolQualification, bytes: &[u8]) -> Result<(), String> {
    let header = parse_header(bytes)?;
    validate_message_shape(bytes, header)?;
    qualification.observe(bytes, parse_source_update(bytes, header)?)
}

#[test]
fn all_four_distinct_probe_flows_and_completed_query_are_required_before_publication() {
    let mut harness = WireHarness::new(pending_qualification());
    for provider in [2_u32, 3, 4, 5, 8] {
        harness
            .pending
            .insert(subscription_context(provider), provider);
    }
    for provider in [2_u32, 3, 4, 5, 8] {
        harness
            .feed(&message(0, subscription_context(provider), 0, 16), -1)
            .unwrap();
    }
    assert!(harness.pending.is_empty());
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::PendingBaseline(_)
    ));

    harness.query(201);
    harness.feed(&source_update(2, 0), -1).unwrap();
    for index in 0..3 {
        harness.feed(&probe_message(index), -1).unwrap();
    }
    harness.feed(&message(0, 201, 0, 16), -1).unwrap();
    assert!(!harness.qualification.complete());
    assert!(!harness.engine.baseline_complete);
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::PendingBaseline(_)
    ));

    harness.feed(&probe_message(3), -1).unwrap();
    assert!(harness.qualification.complete());
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::PendingBaseline(_)
    ));
    harness.query(202);
    harness.feed(&message(0, 202, 0, 16), -1).unwrap();
    assert!(harness.engine.baseline_complete);
    assert!(harness
        .shared
        .lock()
        .unwrap()
        .interval_bytes_by_process
        .is_empty());
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::Held(_)
    ));

    for index in 0..4 {
        let source_ref = index as u64 + 101;
        assert!(harness.qualification.excludes(source_ref));
        assert!(!harness.engine.sources.contains_key(&source_ref));
        // Delayed probe closing updates and drop notifications must remain
        // excluded after finish() has released the held probe sockets.
        let mut closing = probe_message(index);
        closing[14..16].copy_from_slice(&0x0004_u16.to_le_bytes());
        harness.feed(&closing, -1).unwrap();
        let mut removed = message(10_002, 0, 0x0008, 24);
        removed[16..24].copy_from_slice(&source_ref.to_le_bytes());
        harness.feed(&removed, -1).unwrap();
    }
    assert!(harness.shared.lock().unwrap().data_loss.is_none());

    let mut ordinary = source_update(2, 0);
    ordinary[40..48].copy_from_slice(&200_u64.to_le_bytes());
    ordinary[56..64].copy_from_slice(&500_u64.to_le_bytes());
    harness.feed(&ordinary, -1).unwrap();
    harness.query(203);
    harness.feed(&message(0, 203, 0, 16), -1).unwrap();
    let shared = harness.shared.lock().unwrap();
    assert_eq!(shared.interval_bytes_by_process.len(), 1);
    let bytes = shared.interval_bytes_by_process.values().next().unwrap();
    assert_eq!((bytes.received_bps, bytes.transmitted_bps), (77, 44));
    drop(shared);
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::Ready { .. }
    ));
}

#[test]
fn wrong_identity_counter_or_loss_on_the_last_flow_cannot_qualify() {
    for defect in [
        "pid",
        "upid",
        "received",
        "transmitted",
        "discarded",
        "closing",
    ] {
        let mut qualification = pending_qualification();
        for index in 0..3 {
            observe(&mut qualification, &probe_message(index)).unwrap();
        }
        let mut last = probe_message(3);
        match defect {
            "pid" => {
                last[280..284].copy_from_slice(&std::process::id().saturating_add(1).to_le_bytes())
            }
            "upid" => last[152..160].copy_from_slice(&(OWN_UNIQUE_PID + 1).to_le_bytes()),
            "received" => last[40..48].copy_from_slice(&510_u64.to_le_bytes()),
            "transmitted" => last[56..64].copy_from_slice(&258_u64.to_le_bytes()),
            "discarded" => last[24..32].copy_from_slice(&0x8000_0000_u64.to_le_bytes()),
            "closing" => last[14..16].copy_from_slice(&0x0004_u16.to_le_bytes()),
            _ => unreachable!(),
        }
        assert!(
            observe(&mut qualification, &last)
                .unwrap_err()
                .starts_with("nstat_protocol_probe_mismatch:"),
            "{defect}"
        );
        assert!(!qualification.complete(), "{defect}");
        assert!(!qualification.excludes(104), "{defect}");
    }
}

#[test]
fn qualifying_flows_must_have_distinct_stable_source_references() {
    let mut qualification = pending_qualification();
    observe(&mut qualification, &probe_message(0)).unwrap();
    let mut reused = probe_message(1);
    reused[16..24].copy_from_slice(&101_u64.to_le_bytes());
    assert!(
        observe(&mut qualification, &reused).is_err(),
        "two different live tuples cannot share one NStat source reference"
    );
    assert!(!qualification.complete());

    let mut changed = probe_message(0);
    changed[16..24].copy_from_slice(&999_u64.to_le_bytes());
    assert!(observe(&mut qualification, &changed).is_err());
    assert!(!qualification.excludes(999));
}

#[test]
fn unrelated_or_malformed_tuples_do_not_count_as_probe_evidence() {
    for defect in ["family", "length", "address", "local_port", "remote_port"] {
        let mut qualification = pending_qualification();
        for index in 0..4 {
            let mut bytes = probe_message(index);
            let local_offset = 152 + if index < 2 { 124 } else { 56 };
            match defect {
                "family" => bytes[local_offset + 1] = 30,
                "length" => bytes[local_offset] = 0,
                "address" => bytes[local_offset + 7] = 2,
                "local_port" => bytes[local_offset + 2..local_offset + 4].fill(0),
                "remote_port" => bytes[local_offset + 30..local_offset + 32].fill(0),
                _ => unreachable!(),
            }
            observe(&mut qualification, &bytes).unwrap();
        }
        assert!(!qualification.complete(), "{defect}");
        assert!(qualification
            .flows
            .iter()
            .all(|flow| flow.source_ref.is_none()));
        qualification.started_at = Instant::now() - Duration::from_secs(7);
        assert_eq!(
            qualification.check_deadline(),
            Err("nstat_protocol_probe_timed_out".to_string())
        );
    }
}

#[test]
fn incomplete_probe_expires_without_sleep_and_new_session_discards_prior_readiness() {
    let mut qualification = pending_qualification();
    for index in 0..3 {
        observe(&mut qualification, &probe_message(index)).unwrap();
    }
    qualification.started_at = Instant::now() - Duration::from_secs(7);
    assert!(qualification.check_deadline().is_err());

    let mut qualification = pending_qualification();
    for index in 0..4 {
        observe(&mut qualification, &probe_message(index)).unwrap();
    }
    qualification.started_at = Instant::now() - Duration::from_secs(7);
    assert!(qualification.check_deadline().is_ok());
    qualification.finish();
    assert!(qualification.excludes(104));

    let mut harness = WireHarness::new(qualification);
    harness.query(301);
    harness.feed(&message(0, 301, 0, 16), -1).unwrap();
    harness.query(302);
    harness.feed(&message(0, 302, 0, 16), -1).unwrap();
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::Ready { .. }
    ));
    begin_session(&harness.shared, 1);
    harness.qualification = pending_qualification();
    harness.engine = AttributionEngine::default();
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::PendingBaseline(_)
    ));
    harness.query(303);
    harness.feed(&message(0, 303, 0, 16), -1).unwrap();
    assert!(!harness.qualification.complete());
    assert!(matches!(
        harness.sample(),
        NetworkAttributionSample::PendingBaseline(_)
    ));
}
