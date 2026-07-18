use std::collections::{HashMap, HashSet};

use crate::contracts::ProcessSample;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessGeneration {
    pub pid: u32,
    pub start_time_ms: u64,
}

impl ProcessGeneration {
    pub fn from_process(process: &ProcessSample) -> Option<Self> {
        let pid = process.pid.parse().ok()?;
        (process.start_time_ms > 0).then_some(Self {
            pid,
            start_time_ms: process.start_time_ms,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObservedProcessGeneration {
    pub pid: u32,
    pub platform_generation: Option<u64>,
}

impl ObservedProcessGeneration {
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub const fn pid_only(pid: u32) -> Self {
        Self {
            pid,
            platform_generation: None,
        }
    }

    pub const fn platform(pid: u32, platform_generation: u64) -> Self {
        Self {
            pid,
            platform_generation: Some(platform_generation),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessNetworkRates {
    pub received_bps: u64,
    pub transmitted_bps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAttributionSample {
    Ready {
        rates_by_process: HashMap<ObservedProcessGeneration, ProcessNetworkRates>,
    },
    #[cfg_attr(not(windows), allow(dead_code))]
    Partial {
        rates_by_process: HashMap<ObservedProcessGeneration, ProcessNetworkRates>,
        message: String,
    },
    #[cfg_attr(not(windows), allow(dead_code))]
    PendingBaseline(String),
    Held(String),
    Failed(String),
}

impl NetworkAttributionSample {
    #[cfg(test)]
    pub fn ready(
        rates: impl IntoIterator<Item = (ProcessGeneration, ProcessNetworkRates)>,
    ) -> Self {
        Self::Ready {
            rates_by_process: rates
                .into_iter()
                .map(|(generation, rates)| {
                    (ObservedProcessGeneration::pid_only(generation.pid), rates)
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default)]
pub struct NetworkAttributionBinder {
    previous_by_pid: HashMap<u32, ProcessGeneration>,
    previous_platform_generation_by_pid: HashMap<u32, u64>,
}

#[derive(Debug, Default)]
pub struct BoundNetworkRates {
    rates_by_generation: HashMap<ProcessGeneration, ProcessNetworkRates>,
    proven_generations: HashSet<ProcessGeneration>,
}

impl BoundNetworkRates {
    pub fn is_proven(&self, generation: ProcessGeneration) -> bool {
        self.proven_generations.contains(&generation)
    }

    pub fn rates(&self, generation: ProcessGeneration) -> ProcessNetworkRates {
        self.rates_by_generation
            .get(&generation)
            .copied()
            .unwrap_or_default()
    }
}

impl NetworkAttributionBinder {
    pub fn clear(&mut self) {
        self.previous_by_pid.clear();
        self.previous_platform_generation_by_pid.clear();
    }

    pub fn observe(&mut self, processes: &[ProcessSample]) {
        self.previous_by_pid = process_generations_by_pid(processes);
        self.previous_platform_generation_by_pid.clear();
    }

    pub fn bind(
        &mut self,
        processes: &[ProcessSample],
        observed: HashMap<ObservedProcessGeneration, ProcessNetworkRates>,
    ) -> BoundNetworkRates {
        let current_by_pid = process_generations_by_pid(processes);
        let mut bound = BoundNetworkRates::default();
        let mut observed_by_pid = HashMap::<u32, Vec<_>>::new();
        for (identity, rates) in observed {
            observed_by_pid
                .entry(identity.pid)
                .or_default()
                .push((identity, rates));
        }

        let mut next_platform_generation_by_pid = HashMap::new();
        for generation in current_by_pid.values().copied() {
            let runtime_generation_is_stable =
                self.previous_by_pid.get(&generation.pid) == Some(&generation);
            let identities = observed_by_pid
                .get(&generation.pid)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let distinct_platform_generations = identities
                .iter()
                .filter_map(|(identity, _)| identity.platform_generation)
                .collect::<HashSet<_>>();
            if distinct_platform_generations.len() > 1 {
                continue;
            }
            let platform_generation_is_stable = if let Some(platform_generation) =
                distinct_platform_generations.iter().next().copied()
            {
                next_platform_generation_by_pid.insert(generation.pid, platform_generation);
                self.previous_platform_generation_by_pid
                    .get(&generation.pid)
                    == Some(&platform_generation)
            } else if let Some(previous) = self
                .previous_platform_generation_by_pid
                .get(&generation.pid)
                .copied()
            {
                next_platform_generation_by_pid.insert(generation.pid, previous);
                true
            } else {
                true
            };
            if !runtime_generation_is_stable || !platform_generation_is_stable {
                continue;
            }

            let rates =
                identities
                    .iter()
                    .fold(ProcessNetworkRates::default(), |mut total, (_, rates)| {
                        total.received_bps = total.received_bps.saturating_add(rates.received_bps);
                        total.transmitted_bps =
                            total.transmitted_bps.saturating_add(rates.transmitted_bps);
                        total
                    });
            bound.proven_generations.insert(generation);
            if rates != ProcessNetworkRates::default() {
                bound.rates_by_generation.insert(generation, rates);
            }
        }
        self.previous_by_pid = current_by_pid;
        self.previous_platform_generation_by_pid = next_platform_generation_by_pid;
        bound
    }
}

fn process_generations_by_pid(processes: &[ProcessSample]) -> HashMap<u32, ProcessGeneration> {
    processes
        .iter()
        .filter_map(ProcessGeneration::from_process)
        .map(|generation| (generation.pid, generation))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::AccessState;

    fn process(pid: u32, start_time_ms: u64) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms,
            name: "process".to_string(),
            exe: String::new(),
            status: "running".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 0,
            private_bytes: 0,
            virtual_memory_bytes: None,
            io_read_total_bytes: 0,
            io_write_total_bytes: 0,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 0,
            handles: 0,
            access_state: AccessState::Full,
            quality: None,
        }
    }

    #[test]
    fn pid_reuse_never_binds_old_activity_to_the_new_generation() {
        let mut binder = NetworkAttributionBinder::default();
        binder.observe(&[process(42, 1_000)]);

        let bound = binder.bind(
            &[process(42, 2_000)],
            HashMap::from([(
                ObservedProcessGeneration::pid_only(42),
                ProcessNetworkRates {
                    received_bps: 1_024,
                    transmitted_bps: 512,
                },
            )]),
        );

        let reused = ProcessGeneration {
            pid: 42,
            start_time_ms: 2_000,
        };
        assert!(!bound.is_proven(reused));
        assert_eq!(bound.rates(reused), ProcessNetworkRates::default());
    }

    #[test]
    fn stable_generation_binds_pid_only_activity_across_sample_boundaries() {
        let mut binder = NetworkAttributionBinder::default();
        binder.observe(&[process(42, 1_000)]);

        let bound = binder.bind(
            &[process(42, 1_000)],
            HashMap::from([(
                ObservedProcessGeneration::pid_only(42),
                ProcessNetworkRates {
                    received_bps: 1_024,
                    transmitted_bps: 512,
                },
            )]),
        );

        let generation = ProcessGeneration {
            pid: 42,
            start_time_ms: 1_000,
        };
        assert!(bound.is_proven(generation));
        assert_eq!(bound.rates(generation).received_bps, 1_024);
    }

    #[test]
    fn platform_generation_change_rejects_coarse_runtime_identity_collision() {
        let mut binder = NetworkAttributionBinder::default();
        let process = process(42, 1_000);
        binder.observe(std::slice::from_ref(&process));
        let first = binder.bind(
            std::slice::from_ref(&process),
            HashMap::from([(
                ObservedProcessGeneration::platform(42, 100),
                ProcessNetworkRates {
                    received_bps: 100,
                    transmitted_bps: 0,
                },
            )]),
        );
        assert!(!first.is_proven(ProcessGeneration {
            pid: 42,
            start_time_ms: 1_000,
        }));

        let changed = binder.bind(
            std::slice::from_ref(&process),
            HashMap::from([(
                ObservedProcessGeneration::platform(42, 200),
                ProcessNetworkRates {
                    received_bps: 200,
                    transmitted_bps: 0,
                },
            )]),
        );

        assert!(!changed.is_proven(ProcessGeneration {
            pid: 42,
            start_time_ms: 1_000,
        }));

        let stable = binder.bind(
            std::slice::from_ref(&process),
            HashMap::from([(
                ObservedProcessGeneration::platform(42, 200),
                ProcessNetworkRates {
                    received_bps: 300,
                    transmitted_bps: 0,
                },
            )]),
        );
        let generation = ProcessGeneration {
            pid: 42,
            start_time_ms: 1_000,
        };
        assert!(stable.is_proven(generation));
        assert_eq!(stable.rates(generation).received_bps, 300);
    }
}
