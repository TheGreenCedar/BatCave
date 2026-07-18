use super::types::{
    LimitationCode, LimitationEntry, MeasurementDescriptor, MetricObservation, MetricQualityV3,
    MetricScope, MetricSemantic, MetricSourceV3, MetricUnit, NetworkScopeV3,
};
use crate::contracts::{MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource};

pub const QUALITY_CODES: [MetricQualityV3; 5] = [
    MetricQualityV3::Native,
    MetricQualityV3::Estimated,
    MetricQualityV3::Held,
    MetricQualityV3::Partial,
    MetricQualityV3::Unavailable,
];

const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy)]
pub struct SemanticDefinition {
    pub unit: MetricUnit,
    pub sampled_over_interval: bool,
}

pub fn semantic_definition(
    semantic: MetricSemantic,
    scope: MetricScope,
) -> Option<SemanticDefinition> {
    use MetricScope::{Group, Process, System};
    use MetricSemantic::*;
    use MetricUnit::{Bytes, BytesPerSecond, Count, PercentOneCore, PercentSystem};

    let unit = match (scope, semantic) {
        (System, CpuUsage | KernelCpuUsage | LogicalCpuUsage) => PercentSystem,
        (Process | Group, CpuUsage) | (Process, KernelCpuUsage) => PercentOneCore,
        (
            System,
            MemoryUsed
            | MemoryCapacity
            | MemoryAvailable
            | SwapUsed
            | SwapCapacity
            | ProcessWorkingSetMemory
            | ProcessPrivateMemory
            | CommitUsed
            | CommitLimit
            | SystemCache
            | KernelMemory
            | KernelPagedPool
            | KernelNonpagedPool
            | KernelPoolBytes
            | PhysicalDiskReadTotal
            | PhysicalDiskWriteTotal
            | NetworkReceiveTotal
            | NetworkTransmitTotal,
        )
        | (
            Process,
            ResidentMemory | PrivateMemory | VirtualMemory | ReadIoTotal | WriteIoTotal
            | OtherIoTotal,
        )
        | (Group, ResidentMemory) => Bytes,
        (
            System,
            PhysicalDiskReadRate | PhysicalDiskWriteRate | NetworkReceiveRate | NetworkTransmitRate,
        )
        | (
            Process,
            ReadIoRate | WriteIoRate | OtherIoRate | NetworkReceiveRate | NetworkTransmitRate,
        )
        | (Group, ReadWriteIoRate | OtherIoRate | NetworkRate) => BytesPerSecond,
        (
            System,
            ProcessCount
            | DeniedProcessCount
            | PartialProcessCount
            | KernelPoolAllocations
            | KernelPoolFrees,
        )
        | (Process, ThreadCount | HandleCount)
        | (Group, ThreadCount) => Count,
        _ => return None,
    };
    Some(SemanticDefinition {
        unit,
        sampled_over_interval: matches!(unit, PercentOneCore | PercentSystem | BytesPerSecond),
    })
}

pub fn network_scope_definition(
    semantic: MetricSemantic,
    scope: MetricScope,
    source: MetricSourceV3,
) -> Option<NetworkScopeV3> {
    use MetricScope::{Group, Process, System};
    use MetricSemantic::{
        NetworkRate, NetworkReceiveRate, NetworkReceiveTotal, NetworkTransmitRate,
        NetworkTransmitTotal,
    };
    if source == MetricSourceV3::Unknown {
        return None;
    }
    match (scope, semantic) {
        (
            System,
            NetworkReceiveTotal | NetworkTransmitTotal | NetworkReceiveRate | NetworkTransmitRate,
        ) if source == MetricSourceV3::Sysinfo => Some(NetworkScopeV3::AllInterfaceAggregate),
        (
            System,
            NetworkReceiveTotal | NetworkTransmitTotal | NetworkReceiveRate | NetworkTransmitRate,
        ) => Some(NetworkScopeV3::NonLoopbackInterfaceAggregate),
        (Process, NetworkReceiveRate | NetworkTransmitRate) | (Group, NetworkRate) => {
            Some(NetworkScopeV3::IpSocketPayload)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MetricDefinition {
    pub semantic: MetricSemantic,
    pub scope: MetricScope,
    pub unit: MetricUnit,
    pub interval_ms: Option<u32>,
}

impl MetricDefinition {
    pub const fn new(semantic: MetricSemantic, scope: MetricScope, unit: MetricUnit) -> Self {
        Self {
            semantic,
            scope,
            unit,
            interval_ms: None,
        }
    }

    #[allow(dead_code)] // Reserved for collectors that publish completed windows unlike settings.
    pub const fn with_interval_ms(mut self, interval_ms: u32) -> Self {
        self.interval_ms = Some(interval_ms);
        self
    }
}

pub struct CatalogBuilder {
    pub descriptors: Vec<MeasurementDescriptor>,
    pub limitations: Vec<LimitationEntry>,
    sample_interval_ms: u32,
}

impl CatalogBuilder {
    pub fn new(sample_interval_ms: u32) -> Result<Self, String> {
        if sample_interval_ms == 0 {
            return Err("protocol_sample_interval_zero".to_string());
        }
        Ok(Self {
            descriptors: Vec::new(),
            limitations: Vec::new(),
            sample_interval_ms,
        })
    }

    pub fn observation(
        &mut self,
        mut definition: MetricDefinition,
        value: Option<f64>,
        quality: Option<&MetricQualityInfo>,
        sampled_at_ms: Option<u64>,
    ) -> Result<MetricObservation, String> {
        let canonical = semantic_definition(definition.semantic, definition.scope)
            .ok_or_else(|| "protocol_semantic_scope_invalid".to_string())?;
        if definition.unit != canonical.unit {
            return Err("protocol_semantic_unit_invalid".to_string());
        }
        definition.interval_ms = if canonical.sampled_over_interval {
            Some(definition.interval_ms.unwrap_or(self.sample_interval_ms))
        } else if definition.interval_ms.is_some() {
            return Err("protocol_descriptor_interval_invalid".to_string());
        } else {
            None
        };
        if definition.interval_ms == Some(0) {
            return Err("protocol_descriptor_interval_invalid".to_string());
        }
        let missing_source = quality.is_none_or(|quality| quality.source.is_none());
        let source = quality
            .and_then(|quality| quality.source)
            .map(metric_source)
            .unwrap_or(MetricSourceV3::Unknown);
        let descriptor_index = self.descriptor(definition, source)?;
        let mut quality_code = quality
            .map(|quality| metric_quality_code(quality.quality))
            .unwrap_or(metric_quality_code(MetricQuality::Unavailable));
        let mut normalized_value = value.filter(|value| value.is_finite());
        let mut limitation = quality.and_then(|quality| {
            quality.message.as_ref().map(|message| {
                (
                    quality
                        .limitation_code
                        .map(metric_limitation_code)
                        .unwrap_or_else(|| fallback_limitation_code(quality.quality)),
                    message.clone(),
                )
            })
        });

        if missing_source {
            normalized_value = None;
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation = Some((
                LimitationCode::MissingMetadata,
                "Metric source provenance was not reported by the collector.".to_string(),
            ));
        } else if normalized_value.is_none()
            && quality.is_some_and(|quality| {
                matches!(
                    quality.quality,
                    MetricQuality::Native | MetricQuality::Estimated | MetricQuality::Partial
                )
            })
        {
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation.get_or_insert((
                LimitationCode::UnsupportedMetric,
                "Metric value was not reported by the collector.".to_string(),
            ));
        }

        if normalized_value.is_some_and(|value| {
            definition.unit != MetricUnit::PercentOneCore
                && definition.unit != MetricUnit::PercentSystem
                && value > JS_MAX_SAFE_INTEGER as f64
        }) {
            normalized_value = None;
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation = Some((
                LimitationCode::NumericRange,
                "Metric value exceeds the JavaScript safe integer range.".to_string(),
            ));
        }

        let quality_value = QUALITY_CODES
            .get(quality_code as usize)
            .ok_or_else(|| "protocol_quality_code_out_of_range".to_string())?;
        if *quality_value == MetricQualityV3::Unavailable {
            normalized_value = None;
            limitation.get_or_insert((
                LimitationCode::UnsupportedMetric,
                "Metric is unavailable on this source.".to_string(),
            ));
        }

        let pending_baseline = quality.is_some_and(|quality| {
            quality.quality == MetricQuality::Held
                && quality.limitation_code == Some(MetricLimitationCode::PendingBaseline)
        });
        if pending_baseline {
            normalized_value = None;
        }
        let held = quality.is_some_and(|quality| quality.quality == MetricQuality::Held);
        if held
            && !pending_baseline
            && (normalized_value.is_none()
                || quality.and_then(|quality| quality.updated_at_ms).is_none())
        {
            normalized_value = None;
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation = Some((
                LimitationCode::MissingMetadata,
                "Held metric is missing its original observation time.".to_string(),
            ));
        }

        let observed_at_ms = if normalized_value.is_none() {
            None
        } else if held {
            quality.and_then(|quality| quality.updated_at_ms)
        } else {
            quality
                .and_then(|quality| quality.updated_at_ms)
                .or(sampled_at_ms)
        };
        if normalized_value.is_some() && observed_at_ms.is_none() {
            normalized_value = None;
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation = Some((
                LimitationCode::MissingMetadata,
                "Metric observation time was not reported by the collector.".to_string(),
            ));
        }
        if observed_at_ms.is_some_and(|value| value > JS_MAX_SAFE_INTEGER) {
            return Err("protocol_timestamp_out_of_range".to_string());
        }

        let limitation_index = limitation
            .map(|(code, message)| self.limitation(code, message))
            .transpose()?;

        Ok(MetricObservation(
            descriptor_index,
            normalized_value,
            quality_code,
            observed_at_ms,
            limitation_index,
        ))
    }

    pub fn limitation(&mut self, code: LimitationCode, message: String) -> Result<u16, String> {
        if let Some(index) = self
            .limitations
            .iter()
            .position(|entry| entry.code == code && entry.message == message)
        {
            return u16::try_from(index)
                .map_err(|_| "protocol_limitation_catalog_too_large".into());
        }
        let index = u16::try_from(self.limitations.len())
            .map_err(|_| "protocol_limitation_catalog_too_large".to_string())?;
        self.limitations.push(LimitationEntry { code, message });
        Ok(index)
    }

    fn descriptor(
        &mut self,
        definition: MetricDefinition,
        source: MetricSourceV3,
    ) -> Result<u16, String> {
        if let Some(index) = self.descriptors.iter().position(|descriptor| {
            descriptor.semantic == definition.semantic
                && descriptor.scope == definition.scope
                && descriptor.unit == definition.unit
                && descriptor.interval_ms == definition.interval_ms
                && descriptor.network_scope
                    == network_scope_definition(definition.semantic, definition.scope, source)
                && descriptor.source == source
        }) {
            return u16::try_from(index)
                .map_err(|_| "protocol_descriptor_catalog_too_large".into());
        }
        let id = u16::try_from(self.descriptors.len())
            .map_err(|_| "protocol_descriptor_catalog_too_large".to_string())?;
        self.descriptors.push(MeasurementDescriptor {
            id,
            semantic: definition.semantic,
            scope: definition.scope,
            unit: definition.unit,
            interval_ms: definition.interval_ms,
            network_scope: network_scope_definition(definition.semantic, definition.scope, source),
            source,
        });
        Ok(id)
    }
}

pub fn metric_quality_code(quality: MetricQuality) -> u8 {
    match quality {
        MetricQuality::Native => 0,
        MetricQuality::Estimated => 1,
        MetricQuality::Held => 2,
        MetricQuality::Partial => 3,
        MetricQuality::Unavailable => 4,
    }
}

pub fn metric_source(source: MetricSource) -> MetricSourceV3 {
    match source {
        MetricSource::DirectApi => MetricSourceV3::DirectApi,
        MetricSource::Libproc => MetricSourceV3::Libproc,
        MetricSource::Iokit => MetricSourceV3::Iokit,
        MetricSource::Pdh => MetricSourceV3::Pdh,
        MetricSource::InterfaceAggregate => MetricSourceV3::InterfaceAggregate,
        MetricSource::ProcessAggregate => MetricSourceV3::ProcessAggregate,
        MetricSource::Sysinfo => MetricSourceV3::Sysinfo,
        MetricSource::Runtime => MetricSourceV3::Runtime,
        MetricSource::Etw => MetricSourceV3::Etw,
        MetricSource::Nstat => MetricSourceV3::Nstat,
        MetricSource::Procfs => MetricSourceV3::Procfs,
        MetricSource::Ebpf => MetricSourceV3::Ebpf,
        MetricSource::Fixture => MetricSourceV3::Fixture,
    }
}

fn fallback_limitation_code(quality: MetricQuality) -> LimitationCode {
    match quality {
        MetricQuality::Held => LimitationCode::HeldValue,
        MetricQuality::Partial => LimitationCode::PartialCoverage,
        MetricQuality::Unavailable => LimitationCode::UnsupportedMetric,
        MetricQuality::Native | MetricQuality::Estimated => LimitationCode::CollectorFailure,
    }
}

pub fn metric_limitation_code(code: MetricLimitationCode) -> LimitationCode {
    match code {
        MetricLimitationCode::UnsupportedMetric => LimitationCode::UnsupportedMetric,
        MetricLimitationCode::AccessDenied => LimitationCode::AccessDenied,
        MetricLimitationCode::AuthorizationScope => LimitationCode::AuthorizationScope,
        MetricLimitationCode::PartialCoverage => LimitationCode::PartialCoverage,
        MetricLimitationCode::PendingBaseline => LimitationCode::PendingBaseline,
        MetricLimitationCode::HeldValue => LimitationCode::HeldValue,
        MetricLimitationCode::CollectorFailure => LimitationCode::CollectorFailure,
        MetricLimitationCode::DataLoss => LimitationCode::DataLoss,
        MetricLimitationCode::MissingMetadata => LimitationCode::MissingMetadata,
        MetricLimitationCode::GroupPartialCoverage => LimitationCode::GroupPartialCoverage,
        MetricLimitationCode::NumericRange => LimitationCode::NumericRange,
    }
}
