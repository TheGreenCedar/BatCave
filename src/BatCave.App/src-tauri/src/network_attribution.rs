use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessNetworkRates {
    pub received_bps: u64,
    pub transmitted_bps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAttributionSample {
    Ready {
        rates_by_pid: HashMap<u32, ProcessNetworkRates>,
    },
    Held(String),
    Failed(String),
}

impl NetworkAttributionSample {
    #[cfg(test)]
    pub fn ready(rates: impl IntoIterator<Item = (u32, ProcessNetworkRates)>) -> Self {
        Self::Ready {
            rates_by_pid: rates.into_iter().collect(),
        }
    }
}
