use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::time::{Duration, Instant};

use crate::{
    contracts::{
        KernelPoolTag, ProcessMetricQuality, ProcessSample, RuntimeCollectorServiceState,
        RuntimeCollectorServiceStatus, RuntimeReleaseIdentity, SystemMemoryAccounting,
        SystemMetricQuality, SystemMetricsSnapshot,
    },
    telemetry::TelemetrySample,
};

#[cfg(windows)]
use crate::{collector_engine::CollectionFailure, telemetry::TelemetryCollector};

use super::{
    authorization::{authorize_service_identity, VerifiedServicePeer},
    host::current_release_identity,
    protocol::{
        ClientOperationV1, ClientRequestV1, CollectorMetricQualityV1, CollectorProcessQualityV1,
        CollectorSnapshotV1, CollectorSystemQualityV1, LatestSnapshotRequestV1, LatestSnapshotV1,
        NegotiateRequestV1, ReleaseIdentityV1, ServiceFailureCodeV1, ServiceIdentityV1,
        ServiceOutcomeV1, ServiceResponseV1, COLLECTOR_SERVICE_PROTOCOL_VERSION,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientFailureKind {
    NotInstalled,
    Stopped,
    Incompatible,
    Unauthorized,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientFailure {
    pub kind: ClientFailureKind,
    pub detail: String,
    pub service_release: Option<ReleaseIdentityV1>,
}

impl ClientFailure {
    pub(crate) fn new(kind: ClientFailureKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind,
            detail: detail.chars().take(1_024).collect(),
            service_release: None,
        }
    }

    pub(crate) fn with_service_release(mut self, release: ReleaseIdentityV1) -> Self {
        self.service_release = Some(release);
        self
    }
}

pub(crate) trait ClientTransport: Send {
    fn verified_peer(&self) -> &VerifiedServicePeer;
    fn exchange(&mut self, request: &ClientRequestV1) -> Result<ServiceResponseV1, ClientFailure>;
}

pub(crate) struct ServiceClientSession<T: ClientTransport> {
    transport: T,
    identity: ServiceIdentityV1,
    next_request_id: u64,
    last_sample_seq: Option<u64>,
    last_snapshot: Option<CollectorSnapshotV1>,
}

impl<T: ClientTransport> ServiceClientSession<T> {
    pub(crate) fn connect(mut transport: T) -> Result<Self, ClientFailure> {
        let desktop_release = current_release_identity();
        let transport_release = transport.verified_peer().executable_release().clone();
        if transport_release != desktop_release {
            return Err(ClientFailure::new(
                ClientFailureKind::Incompatible,
                "collector_service_desktop_release_incompatible",
            )
            .with_service_release(transport_release));
        }
        let request = ClientRequestV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id: 1,
            operation: ClientOperationV1::Negotiate(NegotiateRequestV1 {
                minimum_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                maximum_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                desktop_release: desktop_release.clone(),
            }),
        };
        let response = transport.exchange(&request)?;
        let outcome = response_outcome(response, request.request_id)?;
        let ServiceOutcomeV1::Negotiated(negotiated) = outcome else {
            return Err(ClientFailure::new(
                ClientFailureKind::Incompatible,
                "collector_service_negotiation_response_invalid",
            ));
        };
        authorize_service_identity(Some(transport.verified_peer()), &negotiated.service)
            .map_err(contract_failure)?;
        if negotiated.negotiated_protocol_version != COLLECTOR_SERVICE_PROTOCOL_VERSION
            || negotiated.service.release != desktop_release
        {
            return Err(ClientFailure::new(
                ClientFailureKind::Incompatible,
                "collector_service_desktop_release_incompatible",
            )
            .with_service_release(negotiated.service.release));
        }

        Ok(Self {
            transport,
            identity: negotiated.service,
            next_request_id: 2,
            last_sample_seq: None,
            last_snapshot: None,
        })
    }

    pub(crate) fn latest_sample(&mut self) -> Result<TelemetrySample, ClientFailure> {
        let request_id = self.take_request_id()?;
        let request = ClientRequestV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id,
            operation: ClientOperationV1::LatestSnapshot(LatestSnapshotRequestV1 {
                after_sample_seq: self.last_sample_seq,
            }),
        };
        let response = self.transport.exchange(&request)?;
        let outcome = response_outcome(response, request_id)?;
        let ServiceOutcomeV1::LatestSnapshot(latest) = outcome else {
            return Err(ClientFailure::new(
                ClientFailureKind::Incompatible,
                "collector_service_snapshot_response_invalid",
            ));
        };
        let snapshot = match latest {
            LatestSnapshotV1::Snapshot(snapshot) => {
                if snapshot.service_instance_id != self.identity.instance_id
                    || self
                        .last_sample_seq
                        .is_some_and(|last| snapshot.sample_seq <= last)
                {
                    return Err(ClientFailure::new(
                        ClientFailureKind::Incompatible,
                        "collector_service_snapshot_sequence_invalid",
                    ));
                }
                let snapshot = *snapshot;
                self.last_sample_seq = Some(snapshot.sample_seq);
                self.last_snapshot = Some(snapshot.clone());
                snapshot
            }
            LatestSnapshotV1::Unchanged(unchanged) => {
                if unchanged.service_instance_id != self.identity.instance_id
                    || self.last_sample_seq != Some(unchanged.sample_seq)
                {
                    return Err(ClientFailure::new(
                        ClientFailureKind::Incompatible,
                        "collector_service_unchanged_sequence_invalid",
                    ));
                }
                self.last_snapshot.clone().ok_or_else(|| {
                    ClientFailure::new(
                        ClientFailureKind::Incompatible,
                        "collector_service_unchanged_without_snapshot",
                    )
                })?
            }
        };

        Ok(sample_from_snapshot(
            snapshot,
            active_status(&self.identity),
        ))
    }

    fn take_request_id(&mut self) -> Result<u64, ClientFailure> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            ClientFailure::new(
                ClientFailureKind::Incompatible,
                "collector_service_request_sequence_exhausted",
            )
        })?;
        Ok(request_id)
    }
}

fn response_outcome(
    response: ServiceResponseV1,
    expected_request_id: u64,
) -> Result<ServiceOutcomeV1, ClientFailure> {
    if response.request_id != expected_request_id {
        return Err(ClientFailure::new(
            ClientFailureKind::Incompatible,
            "collector_service_response_request_mismatch",
        ));
    }
    match response.outcome {
        ServiceOutcomeV1::Error(failure) => Err(ClientFailure::new(
            match failure.code {
                ServiceFailureCodeV1::Incompatible => ClientFailureKind::Incompatible,
                ServiceFailureCodeV1::Unauthorized => ClientFailureKind::Unauthorized,
                ServiceFailureCodeV1::Malformed
                | ServiceFailureCodeV1::Oversized
                | ServiceFailureCodeV1::StaleSequence => ClientFailureKind::Incompatible,
            },
            failure.detail,
        )),
        outcome => Ok(outcome),
    }
}

fn contract_failure(failure: super::protocol::ContractFailure) -> ClientFailure {
    ClientFailure::new(
        match failure.code {
            ServiceFailureCodeV1::Unauthorized => ClientFailureKind::Unauthorized,
            ServiceFailureCodeV1::Incompatible
            | ServiceFailureCodeV1::Malformed
            | ServiceFailureCodeV1::Oversized
            | ServiceFailureCodeV1::StaleSequence => ClientFailureKind::Incompatible,
        },
        failure.detail,
    )
}

pub(crate) fn status_from_failure(
    failure: &ClientFailure,
    previously_active: bool,
) -> RuntimeCollectorServiceStatus {
    let state = match failure.kind {
        ClientFailureKind::NotInstalled => RuntimeCollectorServiceState::NotInstalled,
        ClientFailureKind::Stopped => RuntimeCollectorServiceState::Stopped,
        ClientFailureKind::Incompatible => RuntimeCollectorServiceState::Incompatible,
        ClientFailureKind::Unauthorized => RuntimeCollectorServiceState::Unauthorized,
        ClientFailureKind::Failed if previously_active => RuntimeCollectorServiceState::Recovering,
        ClientFailureKind::Failed => RuntimeCollectorServiceState::Failed,
    };
    let release_identity = failure.service_release.as_ref().map(runtime_release);
    let version = failure
        .service_release
        .as_ref()
        .map(|release| release.app_version.clone());
    RuntimeCollectorServiceStatus {
        state,
        release_identity,
        service_version: version.clone(),
        negotiated_protocol_version: None,
        minimum_desktop_version: (state == RuntimeCollectorServiceState::Incompatible)
            .then_some(version)
            .flatten(),
        instance_id: None,
        last_connected_at_ms: None,
        detail: Some(failure.detail.clone()),
    }
}

fn active_status(identity: &ServiceIdentityV1) -> RuntimeCollectorServiceStatus {
    RuntimeCollectorServiceStatus {
        state: RuntimeCollectorServiceState::Active,
        release_identity: Some(runtime_release(&identity.release)),
        service_version: Some(identity.service_version.clone()),
        negotiated_protocol_version: Some(COLLECTOR_SERVICE_PROTOCOL_VERSION),
        minimum_desktop_version: Some(identity.minimum_desktop_version.clone()),
        instance_id: Some(identity.instance_id.clone()),
        last_connected_at_ms: Some(now_ms()),
        detail: None,
    }
}

fn runtime_release(release: &ReleaseIdentityV1) -> RuntimeReleaseIdentity {
    RuntimeReleaseIdentity {
        app_version: release.app_version.clone(),
        source_commit_sha: release.source_commit_sha.clone(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn sample_from_snapshot(
    snapshot: CollectorSnapshotV1,
    status: RuntimeCollectorServiceStatus,
) -> TelemetrySample {
    let system = snapshot.system;
    TelemetrySample {
        latency_ms: snapshot.collection_latency_ms,
        collector_state: snapshot.collector_state,
        system: SystemMetricsSnapshot {
            cpu_percent: system.cpu_percent,
            kernel_cpu_percent: system.kernel_cpu_percent,
            logical_cpu_percent: system.logical_cpu_percent,
            memory_used_bytes: system.memory_used_bytes,
            memory_total_bytes: system.memory_total_bytes,
            memory_available_bytes: system.memory_available_bytes,
            swap_used_bytes: system.swap_used_bytes,
            swap_total_bytes: system.swap_total_bytes,
            process_count: system.process_count as usize,
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
                .map(|accounting| SystemMemoryAccounting {
                    process_working_set_bytes: accounting.process_working_set_bytes,
                    process_private_bytes: accounting.process_private_bytes,
                    denied_process_count: accounting.denied_process_count as usize,
                    partial_process_count: accounting.partial_process_count as usize,
                    commit_used_bytes: accounting.commit_used_bytes,
                    commit_limit_bytes: accounting.commit_limit_bytes,
                    system_cache_bytes: accounting.system_cache_bytes,
                    kernel_total_bytes: accounting.kernel_total_bytes,
                    kernel_paged_pool_bytes: accounting.kernel_paged_pool_bytes,
                    kernel_nonpaged_pool_bytes: accounting.kernel_nonpaged_pool_bytes,
                    kernel_pool_tags: accounting
                        .kernel_pool_tags
                        .into_iter()
                        .map(|tag| KernelPoolTag {
                            tag: tag.tag,
                            kind: tag.kind,
                            bytes: tag.bytes,
                            allocations: tag.allocations,
                            frees: tag.frees,
                            driver_candidates: tag.driver_candidates,
                            driver_candidates_pending: tag.driver_candidates_pending,
                        })
                        .collect(),
                }),
            quality: system.quality.map(system_quality),
        },
        processes: snapshot
            .processes
            .into_iter()
            .map(|process| ProcessSample {
                pid: process.pid,
                parent_pid: process.parent_pid,
                start_time_ms: process.start_time_ms,
                name: process.name,
                exe: process.exe,
                status: process.status,
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
                quality: process.quality.map(process_quality),
            })
            .collect(),
        warnings: snapshot.warnings,
        collector_service: Some(status),
    }
}

fn metric_quality(value: CollectorMetricQualityV1) -> crate::contracts::MetricQualityInfo {
    crate::contracts::MetricQualityInfo {
        quality: value.quality,
        source: value.source,
        updated_at_ms: value.updated_at_ms,
        age_ms: value.age_ms,
        limitation_code: value.limitation_code,
        message: value.message,
    }
}

fn system_quality(value: CollectorSystemQualityV1) -> SystemMetricQuality {
    SystemMetricQuality {
        cpu: value.cpu.map(metric_quality),
        kernel_cpu: value.kernel_cpu.map(metric_quality),
        logical_cpu: value.logical_cpu.map(metric_quality),
        memory: value.memory.map(metric_quality),
        swap: value.swap.map(metric_quality),
        disk: value.disk.map(metric_quality),
        network: value.network.map(metric_quality),
    }
}

fn process_quality(value: CollectorProcessQualityV1) -> ProcessMetricQuality {
    ProcessMetricQuality {
        cpu: value.cpu.map(metric_quality),
        memory: value.memory.map(metric_quality),
        io: value.io.map(metric_quality),
        other_io: value.other_io.map(metric_quality),
        network: value.network.map(metric_quality),
        threads: value.threads.map(metric_quality),
        handles: value.handles.map(metric_quality),
    }
}

#[cfg(windows)]
pub(crate) struct DesktopCollector {
    service: Option<ServiceClientSession<super::windows_client::WindowsServiceTransport>>,
    fallback: TelemetryCollector,
    retry_at: Instant,
    last_status: Option<RuntimeCollectorServiceStatus>,
}

#[cfg(windows)]
impl DesktopCollector {
    pub(crate) fn new() -> Self {
        Self {
            service: None,
            fallback: TelemetryCollector::for_standard_fallback(),
            retry_at: Instant::now(),
            last_status: None,
        }
    }

    pub(crate) fn collect(&mut self) -> Result<TelemetrySample, CollectionFailure> {
        if self.service.is_none() && Instant::now() >= self.retry_at {
            match super::windows_client::WindowsServiceTransport::connect()
                .and_then(ServiceClientSession::connect)
            {
                Ok(service) => self.service = Some(service),
                Err(failure) => self.note_failure(failure),
            }
        }
        if let Some(service) = &mut self.service {
            match service.latest_sample() {
                Ok(sample) => {
                    self.last_status = sample.collector_service.clone();
                    return Ok(sample);
                }
                Err(failure) => {
                    self.service = None;
                    self.note_failure(failure);
                }
            }
        }

        let mut sample = self.fallback.collect().map_err(|error| {
            if error.contains("lock is poisoned") {
                CollectionFailure::Fatal(error)
            } else {
                CollectionFailure::Unavailable(error)
            }
        })?;
        let status = self.last_status.clone().unwrap_or_else(|| {
            status_from_failure(
                &ClientFailure::new(
                    ClientFailureKind::Failed,
                    "collector_service_connection_unavailable",
                ),
                false,
            )
        });
        if let Some(detail) = &status.detail {
            sample.warnings.push(format!(
                "{detail}; standard-access collector fallback is active"
            ));
        }
        sample.collector_service = Some(status);
        Ok(sample)
    }

    pub(crate) fn process_network_ready(&self) -> Result<bool, String> {
        Ok(false)
    }

    pub(crate) fn retry_process_network(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn note_failure(&mut self, failure: ClientFailure) {
        let previously_active = self
            .last_status
            .as_ref()
            .is_some_and(|status| status.state == RuntimeCollectorServiceState::Active);
        self.last_status = Some(status_from_failure(&failure, previously_active));
        self.retry_at = Instant::now() + Duration::from_secs(5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        collector_service::{
            authorization::VerifiedServicePeer,
            protocol::{NegotiatedV1, ServiceLimitsV1, ServiceOutcomeV1, COLLECTOR_SERVICE_NAME},
        },
        contracts::{RuntimeCollectorServiceState, RuntimeCollectorState},
    };
    use std::collections::VecDeque;

    struct FakeTransport {
        peer: VerifiedServicePeer,
        responses: VecDeque<ServiceResponseV1>,
        requests: Vec<ClientRequestV1>,
    }

    impl ClientTransport for FakeTransport {
        fn verified_peer(&self) -> &VerifiedServicePeer {
            &self.peer
        }

        fn exchange(
            &mut self,
            request: &ClientRequestV1,
        ) -> Result<ServiceResponseV1, ClientFailure> {
            self.requests.push(request.clone());
            self.responses.pop_front().ok_or_else(|| {
                ClientFailure::new(ClientFailureKind::Failed, "fake_transport_empty")
            })
        }
    }

    #[test]
    fn session_requires_transport_verified_matching_release_and_monotonic_snapshot() {
        let identity = identity("instance-1");
        let transport = FakeTransport {
            peer: peer(identity.release.clone()),
            responses: VecDeque::from([
                response(
                    1,
                    ServiceOutcomeV1::Negotiated(NegotiatedV1 {
                        negotiated_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                        service: identity.clone(),
                    }),
                ),
                response(
                    2,
                    ServiceOutcomeV1::LatestSnapshot(LatestSnapshotV1::Snapshot(Box::new(
                        snapshot("instance-1", 7),
                    ))),
                ),
                response(
                    3,
                    ServiceOutcomeV1::LatestSnapshot(LatestSnapshotV1::Unchanged(
                        super::super::protocol::UnchangedSnapshotV1 {
                            service_instance_id: "instance-1".to_string(),
                            sample_seq: 7,
                        },
                    )),
                ),
            ]),
            requests: Vec::new(),
        };

        let mut session = ServiceClientSession::connect(transport).unwrap();
        let first = session.latest_sample().unwrap();
        let held = session.latest_sample().unwrap();
        assert_eq!(first.system.process_count, 0);
        assert_eq!(held.system.cpu_percent, first.system.cpu_percent);
        assert_eq!(
            held.collector_service.unwrap().state,
            RuntimeCollectorServiceState::Active
        );
    }

    #[test]
    fn release_mismatch_is_visible_as_incompatible() {
        let mut service_identity = identity("instance-1");
        service_identity.release.app_version = "different".to_string();
        let failure = ServiceClientSession::connect(FakeTransport {
            peer: peer(service_identity.release.clone()),
            responses: VecDeque::from([response(
                1,
                ServiceOutcomeV1::Negotiated(NegotiatedV1 {
                    negotiated_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                    service: service_identity,
                }),
            )]),
            requests: Vec::new(),
        })
        .err()
        .unwrap();
        assert_eq!(failure.kind, ClientFailureKind::Incompatible);
        let status = status_from_failure(&failure, false);
        assert_eq!(status.state, RuntimeCollectorServiceState::Incompatible);
        assert!(status.service_version.is_some());
        assert!(status.minimum_desktop_version.is_some());
    }

    #[test]
    fn transport_failures_map_to_truthful_fallback_states() {
        for (kind, state) in [
            (
                ClientFailureKind::NotInstalled,
                RuntimeCollectorServiceState::NotInstalled,
            ),
            (
                ClientFailureKind::Stopped,
                RuntimeCollectorServiceState::Stopped,
            ),
            (
                ClientFailureKind::Unauthorized,
                RuntimeCollectorServiceState::Unauthorized,
            ),
            (
                ClientFailureKind::Failed,
                RuntimeCollectorServiceState::Failed,
            ),
        ] {
            assert_eq!(
                status_from_failure(&ClientFailure::new(kind, "detail"), false).state,
                state
            );
        }
        assert_eq!(
            status_from_failure(
                &ClientFailure::new(ClientFailureKind::Failed, "pipe lost"),
                true
            )
            .state,
            RuntimeCollectorServiceState::Recovering
        );
    }

    fn identity(instance_id: &str) -> ServiceIdentityV1 {
        ServiceIdentityV1 {
            service_name: COLLECTOR_SERVICE_NAME.to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            release: current_release_identity(),
            instance_id: instance_id.to_string(),
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            minimum_desktop_version: env!("CARGO_PKG_VERSION").to_string(),
            limits: ServiceLimitsV1::contract(),
        }
    }

    fn peer(release: ReleaseIdentityV1) -> VerifiedServicePeer {
        VerifiedServicePeer::from_transport_verification(20, 30, [1; 32], [2; 32], release).unwrap()
    }

    fn response(request_id: u64, outcome: ServiceOutcomeV1) -> ServiceResponseV1 {
        ServiceResponseV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id,
            outcome,
        }
    }

    fn snapshot(instance_id: &str, sample_seq: u64) -> CollectorSnapshotV1 {
        CollectorSnapshotV1 {
            snapshot_schema_version: super::super::protocol::COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
            service_instance_id: instance_id.to_string(),
            sample_seq,
            sampled_at_ms: 1,
            collection_latency_ms: 2,
            collector_state: RuntimeCollectorState::Healthy,
            system: super::super::protocol::CollectorSystemV1 {
                cpu_percent: 1.0,
                kernel_cpu_percent: 0.0,
                logical_cpu_percent: vec![1.0],
                memory_used_bytes: 1,
                memory_total_bytes: 2,
                memory_available_bytes: Some(1),
                swap_used_bytes: None,
                swap_total_bytes: None,
                process_count: 0,
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
            },
            processes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
