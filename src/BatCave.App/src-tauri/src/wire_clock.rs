use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct MonotonicWireClock {
    origin: Instant,
    wire_origin_ms: u64,
}

impl MonotonicWireClock {
    pub(crate) fn new() -> Self {
        // All desktop wire timestamps share one epoch anchor. Re-sampling wall
        // time for a service status can otherwise put it ahead of publication.
        static CLOCK: OnceLock<MonotonicWireClock> = OnceLock::new();
        CLOCK
            .get_or_init(|| Self {
                origin: Instant::now(),
                wire_origin_ms: crate::telemetry::now_ms(),
            })
            .clone()
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.at_ms(Instant::now())
    }

    pub(crate) fn at_ms(&self, instant: Instant) -> u64 {
        if let Some(elapsed) = instant.checked_duration_since(self.origin) {
            self.wire_origin_ms.saturating_add(duration_ms(elapsed))
        } else {
            self.wire_origin_ms
                .saturating_sub(duration_ms(self.origin.duration_since(instant)))
        }
    }

    #[cfg(test)]
    pub(crate) fn with_origin(origin: Instant, wire_origin_ms: u64) -> Self {
        Self {
            origin,
            wire_origin_ms,
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_users_share_the_exact_epoch_anchor() {
        let runtime = MonotonicWireClock::new();
        let client = MonotonicWireClock::new();
        assert_eq!(runtime.origin, client.origin);
        assert_eq!(runtime.wire_origin_ms, client.wire_origin_ms);
        let observation = Instant::now();
        assert_eq!(runtime.at_ms(observation), client.at_ms(observation));
    }
}
