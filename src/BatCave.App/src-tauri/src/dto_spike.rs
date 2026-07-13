use serde::{Deserialize, Serialize};
use ts_rs::{Config, TS};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    RuntimeSnapshot,
    ProtocolMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum MetricSemantic {
    CpuUsage,
    KernelCpuUsage,
    ResidentMemory,
    PrivateMemory,
    VirtualMemory,
    MemoryUsed,
    MemoryCapacity,
    MemoryAvailable,
    ProcessWorkingSetMemory,
    ProcessPrivateMemory,
    DeniedProcessCount,
    PartialProcessCount,
    UnattributedMemory,
    CommitUsed,
    CommitLimit,
    SystemCache,
    KernelMemory,
    KernelPagedPool,
    KernelNonpagedPool,
    ReadIoTotal,
    WriteIoTotal,
    OtherIoTotal,
    ReadIoRate,
    WriteIoRate,
    OtherIoRate,
    IoRate,
    NetworkReceiveTotal,
    NetworkTransmitTotal,
    NetworkReceiveRate,
    NetworkTransmitRate,
    NetworkRate,
    ProcessCount,
    ThreadCount,
    HandleCount,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum MetricScope {
    Process,
    Group,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum MetricUnit {
    PercentOneCore,
    PercentSystem,
    Bytes,
    BytesPerSecond,
    Count,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum MetricSource {
    DirectApi,
    Pdh,
    Etw,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct MeasurementDescriptor {
    id: u16,
    semantic: MetricSemantic,
    scope: MetricScope,
    unit: MetricUnit,
    interval_ms: Option<u32>,
    source: MetricSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum MetricQuality {
    Native,
    Estimated,
    Held,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
enum AccessState {
    Full,
    Partial,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct MetricObservation(
    u16,
    Option<f64>,
    u8,
    #[ts(type = "number")] u64,
    Option<u16>,
);

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct ProcessDetail {
    stable_id: String,
    pid: String,
    parent_id: Option<String>,
    #[ts(type = "number")]
    start_time_ms: u64,
    display_name: String,
    executable: String,
    status: String,
    access_state: AccessState,
    metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct GroupCoverage {
    included_processes: u32,
    total_processes: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    limitation_indexes: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct GroupDetail {
    stable_id: String,
    display_name: String,
    member_ids: Vec<String>,
    coverage: GroupCoverage,
    metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct SystemDetail {
    stable_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    limitation_indexes: Vec<u16>,
    metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
enum WorkloadDetail {
    Process(ProcessDetail),
    Group(GroupDetail),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct Compatibility {
    minimum_reader_version: u16,
    breaking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct RuntimeEnvelope {
    protocol_version: u16,
    event_kind: EventKind,
    compatibility: Compatibility,
    descriptors: Vec<MeasurementDescriptor>,
    quality_codes: Vec<MetricQuality>,
    limitations: Vec<String>,
    system: SystemDetail,
    workloads: Vec<WorkloadDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct ExistingOptionalWithoutDefault {
    required_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
struct SkippedInternalField {
    visible_name: String,
    #[serde(skip)]
    #[allow(dead_code)]
    internal_only: String,
}

fn generated_typescript() -> String {
    let config = Config::default();
    let declarations = [
        EventKind::decl(&config),
        MetricSemantic::decl(&config),
        MetricScope::decl(&config),
        MetricUnit::decl(&config),
        MetricSource::decl(&config),
        MeasurementDescriptor::decl(&config),
        MetricQuality::decl(&config),
        AccessState::decl(&config),
        MetricObservation::decl(&config),
        ProcessDetail::decl(&config),
        GroupCoverage::decl(&config),
        GroupDetail::decl(&config),
        SystemDetail::decl(&config),
        WorkloadDetail::decl(&config),
        Compatibility::decl(&config),
        RuntimeEnvelope::decl(&config),
        ExistingOptionalWithoutDefault::decl(&config),
        SkippedInternalField::decl(&config),
    ]
    .map(|declaration| declaration.replacen("type ", "export type ", 1));

    format!(
        "// Generated by the issue #66 ts-rs spike; do not edit by hand.\n\n{}\n",
        declarations.join("\n\n")
    )
}

fn representative_envelope() -> RuntimeEnvelope {
    RuntimeEnvelope {
        protocol_version: 3,
        event_kind: EventKind::RuntimeSnapshot,
        compatibility: Compatibility {
            minimum_reader_version: 3,
            breaking: true,
            message: None,
        },
        descriptors: vec![MeasurementDescriptor {
            id: 0,
            semantic: MetricSemantic::CpuUsage,
            scope: MetricScope::Process,
            unit: MetricUnit::PercentOneCore,
            interval_ms: Some(1_000),
            source: MetricSource::DirectApi,
        }],
        quality_codes: vec![
            MetricQuality::Native,
            MetricQuality::Estimated,
            MetricQuality::Held,
            MetricQuality::Partial,
            MetricQuality::Unavailable,
        ],
        limitations: vec![
            "Some protected fields could not be read.".to_string(),
            "Windows reports commit instead of swap.".to_string(),
        ],
        system: SystemDetail {
            stable_id: "system:fixture".to_string(),
            limitation_indexes: vec![1],
            metrics: vec![MetricObservation(0, Some(12.5), 0, 1_720_000_000_000, None)],
        },
        workloads: vec![
            WorkloadDetail::Process(ProcessDetail {
                stable_id: "process:42:1700000000000".to_string(),
                pid: "42".to_string(),
                parent_id: None,
                start_time_ms: 1_700_000_000_000,
                display_name: "fixture.exe".to_string(),
                executable: r"C:\fixture.exe".to_string(),
                status: "running".to_string(),
                access_state: AccessState::Full,
                metrics: vec![MetricObservation(0, Some(12.5), 0, 1_720_000_000_000, None)],
            }),
            WorkloadDetail::Group(GroupDetail {
                stable_id: "group:fixture".to_string(),
                display_name: "Fixture group".to_string(),
                member_ids: vec!["process:42:1700000000000".to_string()],
                coverage: GroupCoverage {
                    included_processes: 1,
                    total_processes: 1,
                    limitation_indexes: vec![0],
                },
                metrics: vec![MetricObservation(
                    0,
                    Some(12.5),
                    3,
                    1_720_000_000_000,
                    Some(0),
                )],
            }),
        ],
    }
}

#[test]
fn generated_typescript_matches_the_checked_fixture() {
    assert_eq!(
        generated_typescript(),
        include_str!("../../src/lib/generated/dto-spike.ts")
    );
}

#[test]
fn serde_optional_behavior_exposes_the_existing_generation_risk() {
    let existing = ExistingOptionalWithoutDefault {
        required_name: "current-pattern".to_string(),
        optional_message: None,
    };
    let clean = Compatibility {
        minimum_reader_version: 2,
        breaking: false,
        message: None,
    };

    assert_eq!(
        serde_json::to_value(existing).expect("serialize existing optional"),
        serde_json::json!({ "required_name": "current-pattern" })
    );
    assert_eq!(
        serde_json::to_value(clean).expect("serialize clean optional"),
        serde_json::json!({
            "minimum_reader_version": 2,
            "breaking": false
        })
    );
}

#[test]
fn selected_transport_serialization_matches_the_checked_json_fixture() {
    let actual = serde_json::to_value(representative_envelope()).expect("serialize spike envelope");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dto-spike.v3.json"))
            .expect("parse checked spike fixture");

    assert_eq!(actual, expected);
}
