use std::{
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    collector_engine::{CollectorEngineHandle, CollectorEvent, CollectorPublication},
    contracts::{
        MetricQualityInfo, ProcessMetricQuality, ProcessSample, SystemMemoryAccounting,
        SystemMetricQuality, SystemMetricsSnapshot,
    },
    telemetry::TelemetrySample,
};

use super::{
    authorization::{AuthorizationSession, AuthorizedOperationV1, VerifiedPeer},
    framing::encode_json_frame,
    protocol::{
        decode_request, incompatible, malformed, oversized, stale_sequence, validate_response,
        CollectorKernelPoolTagV1, CollectorMemoryAccountingV1, CollectorMetricQualityV1,
        CollectorProcessQualityV1, CollectorProcessV1, CollectorSnapshotV1,
        CollectorSystemQualityV1, CollectorSystemV1, ContractFailure, LatestSnapshotV1,
        NegotiatedV1, PingV1, ReleaseIdentityV1, ServiceIdentityV1, ServiceLimitsV1,
        ServiceOutcomeV1, ServiceResponseV1, UnchangedSnapshotV1, COLLECTOR_SERVICE_NAME,
        COLLECTOR_SERVICE_PROTOCOL_VERSION, COLLECTOR_SNAPSHOT_SCHEMA_VERSION, MAX_PROCESS_COUNT,
    },
};

pub(crate) trait SnapshotProvider: Send + Sync {
    fn latest(&self, after_sample_seq: Option<u64>) -> Result<LatestSnapshotV1, ContractFailure>;
}

#[derive(Debug)]
struct ServiceWireClock {
    origin: Instant,
    wire_origin_ms: u64,
}

impl ServiceWireClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            wire_origin_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        }
    }

    fn at_ms(&self, instant: Instant) -> u64 {
        let duration_ms =
            |duration: std::time::Duration| duration.as_millis().try_into().unwrap_or(u64::MAX);
        if let Some(elapsed) = instant.checked_duration_since(self.origin) {
            self.wire_origin_ms.saturating_add(duration_ms(elapsed))
        } else {
            self.wire_origin_ms
                .saturating_sub(duration_ms(self.origin.duration_since(instant)))
        }
    }
}

#[derive(Default)]
struct SnapshotState {
    observed_publication_revision: u64,
    latest: Option<CollectorSnapshotV1>,
}

pub(crate) struct CollectorSnapshotSource {
    collector: CollectorEngineHandle,
    instance_id: String,
    clock: ServiceWireClock,
    state: Mutex<SnapshotState>,
}

impl CollectorSnapshotSource {
    pub(crate) fn new(collector: CollectorEngineHandle, instance_id: String) -> Self {
        Self {
            collector,
            instance_id,
            clock: ServiceWireClock::new(),
            state: Mutex::new(SnapshotState::default()),
        }
    }

    fn refresh_state(&self, state: &mut SnapshotState) -> Result<(), ContractFailure> {
        let Some(publication) = self.collector.snapshot() else {
            return Ok(());
        };
        if publication.revision <= state.observed_publication_revision {
            return Ok(());
        }
        state.observed_publication_revision = publication.revision;
        if let CollectorEvent::Sample(sample) = &publication.event {
            state.latest = Some(snapshot_from_publication(
                &self.instance_id,
                &publication,
                sample,
                &self.clock,
            )?);
        }
        Ok(())
    }
}

impl SnapshotProvider for CollectorSnapshotSource {
    fn latest(&self, after_sample_seq: Option<u64>) -> Result<LatestSnapshotV1, ContractFailure> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| incompatible("collector_service_snapshot_state_unavailable"))?;
        self.refresh_state(&mut state)?;
        let snapshot = state
            .latest
            .as_ref()
            .ok_or_else(|| incompatible("collector_service_snapshot_not_ready"))?;
        match after_sample_seq {
            Some(after) if after > snapshot.sample_seq => {
                Err(stale_sequence("collector_service_requested_sequence_ahead"))
            }
            Some(after) if after == snapshot.sample_seq => {
                Ok(LatestSnapshotV1::Unchanged(UnchangedSnapshotV1 {
                    service_instance_id: snapshot.service_instance_id.clone(),
                    sample_seq: snapshot.sample_seq,
                }))
            }
            _ => Ok(LatestSnapshotV1::Snapshot(Box::new(snapshot.clone()))),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SessionReply {
    pub frame: Vec<u8>,
    pub close: bool,
}

pub(crate) struct ServiceSession {
    identity: ServiceIdentityV1,
    snapshots: Arc<dyn SnapshotProvider>,
    authorization: AuthorizationSession,
}

impl ServiceSession {
    pub(crate) fn new(identity: ServiceIdentityV1, snapshots: Arc<dyn SnapshotProvider>) -> Self {
        Self {
            identity,
            snapshots,
            authorization: AuthorizationSession::default(),
        }
    }

    pub(crate) fn handle_payload(
        &mut self,
        peer: &VerifiedPeer,
        payload: &[u8],
    ) -> Result<SessionReply, ContractFailure> {
        let request = decode_request(payload)?;
        let operation = self.authorization.authorize(Some(peer), &request)?;
        let (outcome, close) = match operation {
            AuthorizedOperationV1::Negotiated {
                protocol_version, ..
            } => (
                ServiceOutcomeV1::Negotiated(NegotiatedV1 {
                    negotiated_protocol_version: protocol_version,
                    service: self.identity.clone(),
                }),
                false,
            ),
            AuthorizedOperationV1::ServiceIdentity { .. } => (
                ServiceOutcomeV1::ServiceIdentity(self.identity.clone()),
                false,
            ),
            AuthorizedOperationV1::LatestSnapshot { request, .. } => (
                ServiceOutcomeV1::LatestSnapshot(self.snapshots.latest(request.after_sample_seq)?),
                false,
            ),
            AuthorizedOperationV1::Ping { nonce, .. } => {
                (ServiceOutcomeV1::Pong(PingV1 { nonce }), false)
            }
            AuthorizedOperationV1::Disconnect { .. } => (ServiceOutcomeV1::Disconnected, true),
        };
        let response = ServiceResponseV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id: request.request_id,
            outcome,
        };
        encode_response(response, close)
    }
}

pub(crate) fn failure_reply(
    request_id: u64,
    failure: &ContractFailure,
) -> Result<SessionReply, ContractFailure> {
    if request_id == 0 {
        return Err(malformed("collector_service_failure_request_id_zero"));
    }
    encode_response(
        ServiceResponseV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id,
            outcome: ServiceOutcomeV1::Error(failure.response()),
        },
        true,
    )
}

pub(crate) fn extract_request_id(payload: &[u8]) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()?
        .get("request_id")?
        .as_u64()
        .filter(|request_id| *request_id > 0)
}

fn encode_response(
    response: ServiceResponseV1,
    close: bool,
) -> Result<SessionReply, ContractFailure> {
    validate_response(&response)?;
    Ok(SessionReply {
        frame: encode_json_frame(&response)?,
        close,
    })
}

pub(crate) fn current_release_identity() -> ReleaseIdentityV1 {
    ReleaseIdentityV1 {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        // The service independently verifies the desktop's Windows ProductVersion.
        // Do not claim a source commit until it is also embedded in and read from
        // the peer executable rather than inherited from the service build.
        source_commit_sha: None,
    }
}

pub(crate) fn service_identity(instance_id: String) -> ServiceIdentityV1 {
    ServiceIdentityV1 {
        service_name: COLLECTOR_SERVICE_NAME.to_string(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        release: current_release_identity(),
        instance_id,
        protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
        minimum_desktop_version: env!("CARGO_PKG_VERSION").to_string(),
        limits: ServiceLimitsV1::contract(),
    }
}

pub(crate) fn new_instance_id() -> String {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:08x}-{started:032x}", std::process::id())
}

fn snapshot_from_publication(
    instance_id: &str,
    publication: &CollectorPublication,
    sample: &TelemetrySample,
    clock: &ServiceWireClock,
) -> Result<CollectorSnapshotV1, ContractFailure> {
    if publication.revision == 0 {
        return Err(malformed("collector_service_publication_sequence_zero"));
    }
    if !publication.collection_latency_ms.is_finite() || publication.collection_latency_ms < 0.0 {
        return Err(malformed("collector_service_collection_latency_invalid"));
    }
    if sample.processes.len() > MAX_PROCESS_COUNT {
        return Err(oversized("collector_service_process_limit_exceeded"));
    }
    if sample.system.process_count != sample.processes.len() {
        return Err(malformed("collector_service_raw_process_count_mismatch"));
    }

    let processes = sample
        .processes
        .iter()
        .map(process_to_wire)
        .collect::<Vec<_>>();
    let snapshot = CollectorSnapshotV1 {
        snapshot_schema_version: COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
        service_instance_id: instance_id.to_string(),
        sample_seq: publication.revision,
        sampled_at_ms: clock.at_ms(publication.completed_at),
        collection_latency_ms: publication
            .collection_latency_ms
            .ceil()
            .max(sample.latency_ms as f64)
            .min(u64::MAX as f64) as u64,
        collector_state: sample.collector_state,
        system: system_to_wire(&sample.system)?,
        processes,
        warnings: sample.warnings.clone(),
    };
    super::protocol::validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn system_to_wire(system: &SystemMetricsSnapshot) -> Result<CollectorSystemV1, ContractFailure> {
    Ok(CollectorSystemV1 {
        cpu_percent: system.cpu_percent,
        kernel_cpu_percent: system.kernel_cpu_percent,
        logical_cpu_percent: system.logical_cpu_percent.clone(),
        memory_used_bytes: system.memory_used_bytes,
        memory_total_bytes: system.memory_total_bytes,
        memory_available_bytes: system.memory_available_bytes,
        swap_used_bytes: system.swap_used_bytes,
        swap_total_bytes: system.swap_total_bytes,
        process_count: system
            .process_count
            .try_into()
            .map_err(|_| oversized("collector_service_process_count_out_of_range"))?,
        disk_read_total_bytes: system.disk_read_total_bytes,
        disk_write_total_bytes: system.disk_write_total_bytes,
        disk_read_bps: system.disk_read_bps,
        disk_write_bps: system.disk_write_bps,
        network_received_total_bytes: system.network_received_total_bytes,
        network_transmitted_total_bytes: system.network_transmitted_total_bytes,
        network_received_bps: system.network_received_bps,
        network_transmitted_bps: system.network_transmitted_bps,
        memory_accounting: system
            .memory_accounting
            .as_ref()
            .map(memory_accounting_to_wire)
            .transpose()?,
        quality: system.quality.as_ref().map(system_quality_to_wire),
    })
}

fn memory_accounting_to_wire(
    accounting: &SystemMemoryAccounting,
) -> Result<CollectorMemoryAccountingV1, ContractFailure> {
    Ok(CollectorMemoryAccountingV1 {
        process_working_set_bytes: accounting.process_working_set_bytes,
        process_private_bytes: accounting.process_private_bytes,
        denied_process_count: accounting
            .denied_process_count
            .try_into()
            .map_err(|_| oversized("collector_service_denied_process_count_out_of_range"))?,
        partial_process_count: accounting
            .partial_process_count
            .try_into()
            .map_err(|_| oversized("collector_service_partial_process_count_out_of_range"))?,
        commit_used_bytes: accounting.commit_used_bytes,
        commit_limit_bytes: accounting.commit_limit_bytes,
        system_cache_bytes: accounting.system_cache_bytes,
        kernel_total_bytes: accounting.kernel_total_bytes,
        kernel_paged_pool_bytes: accounting.kernel_paged_pool_bytes,
        kernel_nonpaged_pool_bytes: accounting.kernel_nonpaged_pool_bytes,
        kernel_pool_tags: accounting
            .kernel_pool_tags
            .iter()
            .map(|tag| CollectorKernelPoolTagV1 {
                tag: tag.tag.clone(),
                kind: tag.kind,
                bytes: tag.bytes,
                allocations: tag.allocations,
                frees: tag.frees,
                driver_candidates: tag.driver_candidates.clone(),
                driver_candidates_pending: tag.driver_candidates_pending,
            })
            .collect(),
    })
}

fn process_to_wire(process: &ProcessSample) -> CollectorProcessV1 {
    CollectorProcessV1 {
        pid: process.pid.clone(),
        parent_pid: process.parent_pid.clone(),
        start_time_ms: process.start_time_ms,
        name: process.name.clone(),
        exe: process.exe.clone(),
        status: process.status.clone(),
        cpu_percent: process.cpu_percent,
        kernel_cpu_percent: process.kernel_cpu_percent,
        memory_bytes: process.memory_bytes,
        private_bytes: process.private_bytes,
        virtual_memory_bytes: process.virtual_memory_bytes,
        io_read_total_bytes: process.io_read_total_bytes,
        io_write_total_bytes: process.io_write_total_bytes,
        other_io_total_bytes: process.other_io_total_bytes,
        io_read_bps: process.io_read_bps,
        io_write_bps: process.io_write_bps,
        other_io_bps: process.other_io_bps,
        network_received_bps: process.network_received_bps,
        network_transmitted_bps: process.network_transmitted_bps,
        threads: process.threads,
        handles: process.handles,
        access_state: process.access_state,
        quality: process.quality.as_ref().map(process_quality_to_wire),
    }
}

fn metric_quality_to_wire(quality: &MetricQualityInfo) -> CollectorMetricQualityV1 {
    CollectorMetricQualityV1 {
        quality: quality.quality,
        source: quality.source,
        updated_at_ms: quality.updated_at_ms,
        age_ms: quality.age_ms,
        limitation_code: quality.limitation_code,
        message: quality.message.clone(),
    }
}

fn system_quality_to_wire(quality: &SystemMetricQuality) -> CollectorSystemQualityV1 {
    CollectorSystemQualityV1 {
        cpu: quality.cpu.as_ref().map(metric_quality_to_wire),
        kernel_cpu: quality.kernel_cpu.as_ref().map(metric_quality_to_wire),
        logical_cpu: quality.logical_cpu.as_ref().map(metric_quality_to_wire),
        memory: quality.memory.as_ref().map(metric_quality_to_wire),
        swap: quality.swap.as_ref().map(metric_quality_to_wire),
        disk: quality.disk.as_ref().map(metric_quality_to_wire),
        network: quality.network.as_ref().map(metric_quality_to_wire),
    }
}

fn process_quality_to_wire(quality: &ProcessMetricQuality) -> CollectorProcessQualityV1 {
    CollectorProcessQualityV1 {
        cpu: quality.cpu.as_ref().map(metric_quality_to_wire),
        memory: quality.memory.as_ref().map(metric_quality_to_wire),
        io: quality.io.as_ref().map(metric_quality_to_wire),
        other_io: quality.other_io.as_ref().map(metric_quality_to_wire),
        network: quality.network.as_ref().map(metric_quality_to_wire),
        threads: quality.threads.as_ref().map(metric_quality_to_wire),
        handles: quality.handles.as_ref().map(metric_quality_to_wire),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collector_engine::{
            CollectionFailure, CollectorEngine, CollectorEngineConfig, RawCollector,
        },
        collector_service::{
            authorization::{VerifiedPeer, VerifiedPeerAuthorization},
            framing::decode_json_payload,
            protocol::{
                ClientOperationV1, ClientRequestV1, LatestSnapshotRequestV1, NegotiateRequestV1,
                ServiceFailureCodeV1,
            },
        },
        contracts::{AccessState, RuntimeCollectorState},
    };
    use std::{collections::VecDeque, time::Duration};

    struct FakeCollector {
        samples: VecDeque<TelemetrySample>,
    }

    impl RawCollector for FakeCollector {
        fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure> {
            self.samples
                .pop_front()
                .ok_or_else(|| CollectionFailure::Unavailable("empty".to_string()))
        }
    }

    #[test]
    fn raw_publications_map_to_monotonic_snapshot_and_unchanged_responses() {
        let engine = CollectorEngine::start(
            Box::new(FakeCollector {
                samples: VecDeque::from([sample(10)]),
            }),
            CollectorEngineConfig {
                interval: Duration::from_secs(1),
                metric_window: Duration::from_secs(60),
                paused: false,
                automatic: false,
            },
            Arc::new(|| {}),
        )
        .unwrap();
        engine.handle().refresh_now().unwrap();
        let source = CollectorSnapshotSource::new(engine.handle(), "instance".to_string());

        let first = source.latest(None).unwrap();
        let seq = match first {
            LatestSnapshotV1::Snapshot(snapshot) => {
                assert_eq!(snapshot.system.process_count, 1);
                assert_eq!(snapshot.processes[0].pid, "10");
                snapshot.sample_seq
            }
            LatestSnapshotV1::Unchanged(_) => panic!("first read must publish a snapshot"),
        };
        assert!(matches!(
            source.latest(Some(seq)).unwrap(),
            LatestSnapshotV1::Unchanged(UnchangedSnapshotV1 { sample_seq, .. }) if sample_seq == seq
        ));
        assert_eq!(
            source.latest(Some(seq + 1)).unwrap_err().code,
            ServiceFailureCodeV1::StaleSequence
        );
        engine.shutdown().unwrap();
    }

    #[test]
    fn service_instances_have_bounded_nonempty_identity() {
        let instance = new_instance_id();
        assert!(!instance.is_empty());
        assert!(instance.len() < 128);
        super::super::protocol::validate_service_identity(&service_identity(instance)).unwrap();
    }

    #[test]
    fn session_requires_negotiation_and_binds_read_only_operations_to_peer() {
        let identity = service_identity("instance".to_string());
        let snapshots = Arc::new(StaticSnapshot(sample_snapshot("instance", 4)));
        let mut session = ServiceSession::new(identity.clone(), snapshots);
        let peer = peer(10);

        let pre_negotiate = request(1, ClientOperationV1::ServiceIdentity);
        assert_eq!(
            session
                .handle_payload(&peer, &serde_json::to_vec(&pre_negotiate).unwrap())
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Incompatible
        );

        let negotiate = request(
            2,
            ClientOperationV1::Negotiate(NegotiateRequestV1 {
                minimum_protocol_version: 1,
                maximum_protocol_version: 1,
                desktop_release: current_release_identity(),
            }),
        );
        let negotiated = session
            .handle_payload(&peer, &serde_json::to_vec(&negotiate).unwrap())
            .unwrap();
        assert!(!negotiated.close);
        let response: ServiceResponseV1 = decode_frame(&negotiated.frame);
        assert!(matches!(response.outcome, ServiceOutcomeV1::Negotiated(_)));

        let latest = request(
            3,
            ClientOperationV1::LatestSnapshot(LatestSnapshotRequestV1 {
                after_sample_seq: None,
            }),
        );
        let response: ServiceResponseV1 = decode_frame(
            &session
                .handle_payload(&peer, &serde_json::to_vec(&latest).unwrap())
                .unwrap()
                .frame,
        );
        assert!(matches!(
            response.outcome,
            ServiceOutcomeV1::LatestSnapshot(_)
        ));

        let disconnected = session
            .handle_payload(
                &peer,
                &serde_json::to_vec(&request(4, ClientOperationV1::Disconnect)).unwrap(),
            )
            .unwrap();
        assert!(disconnected.close);
    }

    #[test]
    fn malformed_requests_get_only_bounded_request_bound_failure_frames() {
        let payload = br#"{"request_id":77,"operation":{"kind":"run_command"}}"#;
        let failure = decode_request(payload).unwrap_err();
        let reply = failure_reply(extract_request_id(payload).unwrap(), &failure).unwrap();
        assert!(reply.close);
        let response: ServiceResponseV1 = decode_frame(&reply.frame);
        assert_eq!(response.request_id, 77);
        assert!(matches!(response.outcome, ServiceOutcomeV1::Error(_)));
        assert_eq!(extract_request_id(br#"{"request_id":0}"#), None);
    }

    #[test]
    fn conversion_rejects_raw_process_count_drift_before_wire_publication() {
        let mut sample = sample(10);
        sample.system.process_count = 2;
        let publication = CollectorPublication {
            revision: 1,
            completed_at: Instant::now(),
            event: CollectorEvent::Sample(Arc::new(sample.clone())),
            collection_latency_ms: 1.0,
            cadence: Default::default(),
        };
        assert_eq!(
            snapshot_from_publication("instance", &publication, &sample, &ServiceWireClock::new(),)
                .unwrap_err()
                .detail,
            "collector_service_raw_process_count_mismatch"
        );
    }

    struct StaticSnapshot(CollectorSnapshotV1);

    impl SnapshotProvider for StaticSnapshot {
        fn latest(
            &self,
            after_sample_seq: Option<u64>,
        ) -> Result<LatestSnapshotV1, ContractFailure> {
            if after_sample_seq == Some(self.0.sample_seq) {
                Ok(LatestSnapshotV1::Unchanged(UnchangedSnapshotV1 {
                    service_instance_id: self.0.service_instance_id.clone(),
                    sample_seq: self.0.sample_seq,
                }))
            } else {
                Ok(LatestSnapshotV1::Snapshot(Box::new(self.0.clone())))
            }
        }
    }

    fn request(request_id: u64, operation: ClientOperationV1) -> ClientRequestV1 {
        ClientRequestV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id,
            operation,
        }
    }

    fn peer(process_id: u32) -> VerifiedPeer {
        VerifiedPeer::from_transport_verification(
            process_id,
            20,
            1,
            [1; 32],
            [2; 32],
            current_release_identity(),
            VerifiedPeerAuthorization::CollectorClient,
        )
        .unwrap()
    }

    fn decode_frame<T: serde::de::DeserializeOwned>(frame: &[u8]) -> T {
        let length = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        decode_json_payload(&frame[4..4 + length]).unwrap()
    }

    fn sample(pid: u32) -> TelemetrySample {
        let process = ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1,
            name: "BatCave".to_string(),
            exe: r"C:\Program Files\BatCave Monitor\batcave-monitor.exe".to_string(),
            status: "Run".to_string(),
            cpu_percent: 1.0,
            kernel_cpu_percent: Some(0.2),
            memory_bytes: 100,
            private_bytes: 90,
            virtual_memory_bytes: Some(200),
            io_read_total_bytes: 10,
            io_write_total_bytes: 20,
            other_io_total_bytes: Some(1),
            io_read_bps: 1,
            io_write_bps: 2,
            other_io_bps: Some(1),
            network_received_bps: Some(3),
            network_transmitted_bps: Some(4),
            threads: 2,
            handles: 3,
            access_state: AccessState::Full,
            quality: None,
        };
        TelemetrySample {
            latency_ms: 1,
            collector_state: RuntimeCollectorState::Healthy,
            system: empty_system(1),
            processes: vec![process],
            warnings: Vec::new(),
            collector_service: None,
            source_provenance: None,
            standard_fallback_process_etw_disabled: false,
        }
    }

    fn empty_system(process_count: usize) -> SystemMetricsSnapshot {
        SystemMetricsSnapshot {
            cpu_percent: 1.0,
            kernel_cpu_percent: 0.1,
            logical_cpu_percent: vec![1.0],
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            memory_available_bytes: Some(1),
            swap_used_bytes: Some(0),
            swap_total_bytes: Some(0),
            process_count,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            disk_read_bps: 0,
            disk_write_bps: 0,
            network_received_total_bytes: 0,
            network_transmitted_total_bytes: 0,
            network_received_bps: 0,
            network_transmitted_bps: 0,
            memory_accounting: None,
            quality: None,
        }
    }

    fn sample_snapshot(instance_id: &str, sample_seq: u64) -> CollectorSnapshotV1 {
        let sample = sample(10);
        CollectorSnapshotV1 {
            snapshot_schema_version: COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
            service_instance_id: instance_id.to_string(),
            sample_seq,
            sampled_at_ms: 1,
            collection_latency_ms: 1,
            collector_state: sample.collector_state,
            system: system_to_wire(&sample.system).unwrap(),
            processes: sample.processes.iter().map(process_to_wire).collect(),
            warnings: Vec::new(),
        }
    }
}
