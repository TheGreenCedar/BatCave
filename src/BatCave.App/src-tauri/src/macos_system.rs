use crate::contracts::{
    MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource, ProcessSample,
    SystemMetricQuality, SystemMetricsSnapshot,
};

#[derive(Debug, Default)]
pub struct MacosSystemCollector;

impl MacosSystemCollector {
    pub fn new() -> Self {
        Self
    }

    pub fn enrich(&mut self, snapshot: &mut SystemMetricsSnapshot, _processes: &[ProcessSample]) {
        snapshot.disk_read_total_bytes = 0;
        snapshot.disk_write_total_bytes = 0;
        snapshot.disk_read_bps = 0;
        snapshot.disk_write_bps = 0;
        let disk_quality = MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Runtime)
            .with_limitation(
                MetricLimitationCode::UnsupportedMetric,
                "Physical-disk throughput is unavailable because the macOS collector has no trusted device-level source. Process read/write I/O is kept separate.",
            );

        snapshot.quality = Some(SystemMetricQuality {
            cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            kernel_cpu: Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Sysinfo)
                    .with_limitation(
                        MetricLimitationCode::UnsupportedMetric,
                        "Kernel CPU is unavailable from the macOS system collector.",
                    ),
            ),
            logical_cpu: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Sysinfo,
            )),
            swap: Some(MetricQualityInfo::new(
                MetricQuality::Estimated,
                MetricSource::Sysinfo,
            )),
            disk: Some(disk_quality),
            network: Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Sysinfo,
            )),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{AccessState, ProcessMetricQuality, SystemMetricsSnapshot};

    fn system() -> SystemMetricsSnapshot {
        SystemMetricsSnapshot {
            cpu_percent: 0.0,
            kernel_cpu_percent: 0.0,
            logical_cpu_percent: vec![],
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            memory_available_bytes: Some(1),
            swap_used_bytes: Some(0),
            swap_total_bytes: Some(0),
            process_count: 1,
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

    fn process(io_quality: MetricQuality) -> ProcessSample {
        ProcessSample {
            pid: "1".to_string(),
            parent_pid: None,
            start_time_ms: 0,
            name: "test".to_string(),
            exe: String::new(),
            status: String::new(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 0,
            private_bytes: 0,
            virtual_memory_bytes: None,
            io_read_total_bytes: 10,
            io_write_total_bytes: 20,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 0,
            handles: 0,
            access_state: AccessState::Partial,
            quality: Some(ProcessMetricQuality {
                io: Some(MetricQualityInfo::new(io_quality, MetricSource::DirectApi)),
                ..ProcessMetricQuality::default()
            }),
        }
    }

    #[test]
    fn process_io_never_populates_physical_system_disk() {
        let mut snapshot = system();
        snapshot.disk_read_total_bytes = 99;
        snapshot.disk_write_total_bytes = 101;
        snapshot.disk_read_bps = 7;
        snapshot.disk_write_bps = 8;
        MacosSystemCollector::new().enrich(&mut snapshot, &[process(MetricQuality::Native)]);
        assert_eq!(snapshot.disk_read_total_bytes, 0);
        assert_eq!(snapshot.disk_write_total_bytes, 0);
        assert_eq!(snapshot.disk_read_bps, 0);
        assert_eq!(snapshot.disk_write_bps, 0);
        let disk = snapshot.quality.unwrap().disk.unwrap();
        assert_eq!(disk.quality, MetricQuality::Unavailable);
        assert_eq!(disk.source, Some(MetricSource::Runtime));
        assert!(disk
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Process read/write I/O is kept separate")));
    }

    #[test]
    fn disk_is_unavailable_without_device_level_source() {
        let mut snapshot = system();
        MacosSystemCollector::new().enrich(&mut snapshot, &[process(MetricQuality::Unavailable)]);
        let disk = snapshot.quality.unwrap().disk.unwrap();
        assert_eq!(disk.quality, MetricQuality::Unavailable);
        assert_eq!(disk.source, Some(MetricSource::Runtime));
    }
}
