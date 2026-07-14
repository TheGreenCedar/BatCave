use serde::{de::DeserializeOwned, Serialize};

use super::protocol::{malformed, oversized, ContractFailure, MAX_FRAME_BYTES};

pub(crate) const FRAME_HEADER_BYTES: usize = std::mem::size_of::<u32>();
pub(crate) const MAX_FRAMES_PER_BATCH: usize = 64;

pub(crate) fn encode_json_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractFailure> {
    let payload = serde_json::to_vec(value)
        .map_err(|_| malformed("collector_service_frame_serialize_failed"))?;
    if payload.is_empty() {
        return Err(malformed("collector_service_frame_empty"));
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(oversized("collector_service_frame_too_large"));
    }
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| oversized("collector_service_frame_length_out_of_range"))?;
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub(crate) fn decode_json_payload<T: DeserializeOwned>(
    payload: &[u8],
) -> Result<T, ContractFailure> {
    if payload.is_empty() {
        return Err(malformed("collector_service_frame_empty"));
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(oversized("collector_service_frame_too_large"));
    }
    serde_json::from_slice(payload)
        .map_err(|_| malformed("collector_service_frame_payload_malformed"))
}

#[derive(Debug, Default)]
pub(crate) struct FrameDecoder {
    header: [u8; FRAME_HEADER_BYTES],
    header_len: usize,
    expected_payload_len: Option<usize>,
    payload: Vec<u8>,
}

impl FrameDecoder {
    pub(crate) fn push(&mut self, mut bytes: &[u8]) -> Result<Vec<Vec<u8>>, ContractFailure> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            if self.expected_payload_len.is_none() {
                let header_remaining = FRAME_HEADER_BYTES - self.header_len;
                let copied = header_remaining.min(bytes.len());
                self.header[self.header_len..self.header_len + copied]
                    .copy_from_slice(&bytes[..copied]);
                self.header_len += copied;
                bytes = &bytes[copied..];
                if self.header_len < FRAME_HEADER_BYTES {
                    continue;
                }

                let expected = u32::from_le_bytes(self.header) as usize;
                if expected == 0 {
                    self.reset();
                    return Err(malformed("collector_service_frame_empty"));
                }
                if expected > MAX_FRAME_BYTES {
                    self.reset();
                    return Err(oversized("collector_service_frame_too_large"));
                }
                self.expected_payload_len = Some(expected);
                self.payload = Vec::with_capacity(expected);
            }

            let expected = self
                .expected_payload_len
                .expect("frame payload length is present after header parsing");
            let remaining = expected - self.payload.len();
            let copied = remaining.min(bytes.len());
            self.payload.extend_from_slice(&bytes[..copied]);
            bytes = &bytes[copied..];

            if self.payload.len() == expected {
                frames.push(std::mem::take(&mut self.payload));
                self.header = [0; FRAME_HEADER_BYTES];
                self.header_len = 0;
                self.expected_payload_len = None;
                if frames.len() == MAX_FRAMES_PER_BATCH && !bytes.is_empty() {
                    self.reset();
                    return Err(oversized("collector_service_frame_batch_too_large"));
                }
            }
        }
        Ok(frames)
    }

    pub(crate) fn buffered_bytes(&self) -> usize {
        self.header_len + self.payload.len()
    }

    fn reset(&mut self) {
        self.header = [0; FRAME_HEADER_BYTES];
        self.header_len = 0;
        self.expected_payload_len = None;
        self.payload.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collector_service::protocol::{
            validate_response, CollectorProcessV1, CollectorSnapshotV1, CollectorSystemV1,
            LatestSnapshotV1, ServiceOutcomeV1, ServiceResponseV1,
            COLLECTOR_SERVICE_PROTOCOL_VERSION, COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
            MAX_PROCESS_COUNT,
        },
        contracts::{AccessState, RuntimeCollectorState},
    };

    #[test]
    fn fragmented_and_coalesced_frames_decode_without_unbounded_buffering() {
        let first = encode_json_frame(&serde_json::json!({"frame": 1})).unwrap();
        let second = encode_json_frame(&serde_json::json!({"frame": 2})).unwrap();
        let joined = [first.clone(), second.clone()].concat();
        let mut decoder = FrameDecoder::default();

        assert!(decoder.push(&joined[..2]).unwrap().is_empty());
        assert_eq!(decoder.buffered_bytes(), 2);
        assert!(decoder.push(&joined[2..7]).unwrap().is_empty());
        let frames = decoder.push(&joined[7..]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(
            decode_json_payload::<serde_json::Value>(&frames[0]).unwrap(),
            serde_json::json!({"frame": 1})
        );
        assert_eq!(
            decode_json_payload::<serde_json::Value>(&frames[1]).unwrap(),
            serde_json::json!({"frame": 2})
        );
        assert_eq!(decoder.buffered_bytes(), 0);
    }

    #[test]
    fn empty_and_oversized_lengths_fail_before_payload_allocation() {
        let mut decoder = FrameDecoder::default();
        assert_eq!(
            decoder.push(&0_u32.to_le_bytes()).unwrap_err().code,
            crate::collector_service::protocol::ServiceFailureCodeV1::Malformed
        );
        assert_eq!(decoder.buffered_bytes(), 0);

        let oversized = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_le_bytes();
        assert_eq!(
            decoder.push(&oversized).unwrap_err().code,
            crate::collector_service::protocol::ServiceFailureCodeV1::Oversized
        );
        assert_eq!(decoder.buffered_bytes(), 0);
    }

    #[test]
    fn a_single_decode_batch_is_bounded() {
        let frame = encode_json_frame(&serde_json::json!({"ok": true})).unwrap();
        let batch = frame.repeat(MAX_FRAMES_PER_BATCH + 1);
        let failure = FrameDecoder::default().push(&batch).unwrap_err();
        assert_eq!(
            failure.code,
            crate::collector_service::protocol::ServiceFailureCodeV1::Oversized
        );
    }

    #[test]
    fn malformed_json_and_serialized_oversize_are_structured_failures() {
        assert_eq!(
            decode_json_payload::<serde_json::Value>(b"{not-json")
                .unwrap_err()
                .code,
            crate::collector_service::protocol::ServiceFailureCodeV1::Malformed
        );
        let too_large = "x".repeat(MAX_FRAME_BYTES);
        assert_eq!(
            encode_json_frame(&too_large).unwrap_err().code,
            crate::collector_service::protocol::ServiceFailureCodeV1::Oversized
        );
    }

    #[test]
    fn representative_maximum_process_snapshot_fits_the_frame_budget() {
        let process = sample_process();
        let processes = vec![process; MAX_PROCESS_COUNT];
        let response = ServiceResponseV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id: 3,
            outcome: ServiceOutcomeV1::LatestSnapshot(LatestSnapshotV1::Snapshot(Box::new(
                CollectorSnapshotV1 {
                    snapshot_schema_version: COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
                    service_instance_id: "instance-1".to_string(),
                    sample_seq: 1,
                    sampled_at_ms: 1_700_000_000_000,
                    collection_latency_ms: 15,
                    collector_state: RuntimeCollectorState::Healthy,
                    system: system(MAX_PROCESS_COUNT as u32),
                    processes,
                    warnings: Vec::new(),
                },
            ))),
        };

        validate_response(&response).unwrap();
        let encoded = encode_json_frame(&response).unwrap();
        assert!(encoded.len() <= MAX_FRAME_BYTES + FRAME_HEADER_BYTES);
        assert!(encoded.len() > 1_000_000);
    }

    fn system(process_count: u32) -> CollectorSystemV1 {
        CollectorSystemV1 {
            cpu_percent: 10.0,
            kernel_cpu_percent: 2.0,
            logical_cpu_percent: vec![10.0, 11.0],
            memory_used_bytes: 1_000,
            memory_total_bytes: 2_000,
            memory_available_bytes: Some(1_000),
            swap_used_bytes: Some(0),
            swap_total_bytes: Some(0),
            process_count,
            disk_read_total_bytes: 10,
            disk_write_total_bytes: 20,
            disk_read_bps: 1,
            disk_write_bps: 2,
            network_received_total_bytes: 30,
            network_transmitted_total_bytes: 40,
            network_received_bps: 3,
            network_transmitted_bps: 4,
            memory_accounting: None,
            quality: None,
        }
    }

    fn sample_process() -> CollectorProcessV1 {
        CollectorProcessV1 {
            pid: "1234".to_string(),
            parent_pid: Some("1".to_string()),
            start_time_ms: 1_700_000_000_000,
            name: "batcave-monitor".to_string(),
            exe: r"C:\Program Files\BatCave Monitor\batcave-monitor.exe".to_string(),
            status: "Run".to_string(),
            cpu_percent: 1.0,
            kernel_cpu_percent: Some(0.2),
            memory_bytes: 100,
            private_bytes: 90,
            virtual_memory_bytes: Some(200),
            io_read_total_bytes: 10,
            io_write_total_bytes: 20,
            other_io_total_bytes: Some(5),
            io_read_bps: 1,
            io_write_bps: 2,
            other_io_bps: Some(1),
            network_received_bps: Some(3),
            network_transmitted_bps: Some(4),
            threads: 5,
            handles: 6,
            access_state: AccessState::Full,
            quality: None,
        }
    }
}
