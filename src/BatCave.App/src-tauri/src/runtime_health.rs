use crate::contracts::{
    RuntimeCollectorState, RuntimeEngineState, RuntimeFreshness, RuntimeHealthReason,
    RuntimeSnapshot,
};

/// Evaluate the immutable publication using the runtime's monotonic wire clock.
/// A settings publication and a heartbeat are not successful telemetry samples.
pub(crate) fn evaluate_snapshot_health(snapshot: &mut RuntimeSnapshot, now_ms: u64) {
    use RuntimeHealthReason::*;
    let now_ms = now_ms.max(snapshot.published_at_ms);
    let health = &mut snapshot.health;
    health.updated_at_ms = now_ms;
    health.reason_codes.retain(|reason| {
        !matches!(
            reason,
            EngineFatal | HeartbeatStale | PublicationStale | SampleStale
        )
    });

    health.freshness = if health.engine_state == Some(RuntimeEngineState::Fatal) {
        health.reason_codes.push(EngineFatal);
        health.status_summary = "Sampling engine stopped after a fatal error.".to_string();
        RuntimeFreshness::Stale
    } else if snapshot.settings.paused || health.engine_state == Some(RuntimeEngineState::Paused) {
        RuntimeFreshness::Paused
    } else if snapshot.sampled_at_ms.is_none()
        || health.engine_state == Some(RuntimeEngineState::Starting)
    {
        RuntimeFreshness::Starting
    } else {
        // A slow prior collection must not make a stalled collector look current.
        let max_age_ms = u64::from(snapshot.settings.sample_interval_ms.clamp(500, 5_000)) * 2;
        if health.engine_state == Some(RuntimeEngineState::Running) {
            if health
                .last_heartbeat_at_ms
                .is_none_or(|at| now_ms.saturating_sub(at) > max_age_ms)
            {
                health.reason_codes.push(HeartbeatStale);
            }
            if now_ms.saturating_sub(snapshot.published_at_ms) > max_age_ms {
                health.reason_codes.push(PublicationStale);
            }
        }
        if snapshot
            .sampled_at_ms
            .is_none_or(|at| now_ms.saturating_sub(at) > max_age_ms)
        {
            health.reason_codes.push(SampleStale);
        }
        if health
            .reason_codes
            .iter()
            .any(|reason| matches!(reason, HeartbeatStale | PublicationStale | SampleStale))
        {
            health.status_summary = if health.reason_codes.contains(&HeartbeatStale) {
                "Sampling engine heartbeat is stale."
            } else if health.reason_codes.contains(&PublicationStale) {
                "Snapshot publication is stale."
            } else {
                "Telemetry sample is stale."
            }
            .to_string();
            RuntimeFreshness::Stale
        } else if health.collector_state == Some(RuntimeCollectorState::Unavailable) {
            RuntimeFreshness::Stale
        } else {
            RuntimeFreshness::Live
        }
    };
    health.degraded = !health.reason_codes.is_empty();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> RuntimeSnapshot {
        let mut snapshot = crate::protocol::test_runtime_snapshot();
        snapshot.published_at_ms = 10_000;
        snapshot.sampled_at_ms = Some(10_000);
        snapshot.settings.sample_interval_ms = 1_000;
        snapshot.settings.paused = false;
        snapshot.health.engine_state = Some(RuntimeEngineState::Running);
        snapshot.health.last_heartbeat_at_ms = Some(10_000);
        snapshot.health.reason_codes.clear();
        snapshot.health.collector_state = Some(RuntimeCollectorState::Healthy);
        snapshot
    }

    #[test]
    fn publications_do_not_refresh_stalled_samples_or_extend_their_budget() {
        for cadence in [500, 1_000, 5_000] {
            let mut snapshot = snapshot();
            snapshot.settings.sample_interval_ms = cadence;
            snapshot.health.collection_latency_ms = Some(600_000.0);
            let boundary = 10_000 + u64::from(cadence) * 2;
            snapshot.published_at_ms = boundary;
            snapshot.health.last_heartbeat_at_ms = Some(boundary);
            evaluate_snapshot_health(&mut snapshot, boundary);
            assert_eq!(snapshot.health.freshness, RuntimeFreshness::Live);
            snapshot.published_at_ms += 1;
            snapshot.health.last_heartbeat_at_ms = Some(boundary + 1);
            evaluate_snapshot_health(&mut snapshot, boundary + 1);
            assert_eq!(snapshot.health.freshness, RuntimeFreshness::Stale);
            assert_eq!(
                snapshot.health.reason_codes,
                vec![RuntimeHealthReason::SampleStale]
            );
        }
    }

    #[test]
    fn recovery_removes_temporal_reasons_and_preserves_independent_faults() {
        let mut snapshot = snapshot();
        snapshot
            .health
            .reason_codes
            .push(RuntimeHealthReason::PersistenceUnavailable);
        evaluate_snapshot_health(&mut snapshot, 13_000);
        assert!(snapshot
            .health
            .reason_codes
            .contains(&RuntimeHealthReason::HeartbeatStale));
        assert!(snapshot
            .health
            .reason_codes
            .contains(&RuntimeHealthReason::PublicationStale));
        assert!(snapshot
            .health
            .reason_codes
            .contains(&RuntimeHealthReason::SampleStale));
        snapshot.published_at_ms = 13_100;
        snapshot.sampled_at_ms = Some(13_100);
        snapshot.health.last_heartbeat_at_ms = Some(13_100);
        evaluate_snapshot_health(&mut snapshot, 13_100);
        assert_eq!(snapshot.health.freshness, RuntimeFreshness::Live);
        assert_eq!(
            snapshot.health.reason_codes,
            vec![RuntimeHealthReason::PersistenceUnavailable]
        );
        assert!(snapshot.health.degraded);
    }

    #[test]
    fn pause_and_start_do_not_fabricate_samples_and_fatal_wins() {
        let mut snapshot = snapshot();
        snapshot.sampled_at_ms = None;
        snapshot.health.engine_state = Some(RuntimeEngineState::Starting);
        evaluate_snapshot_health(&mut snapshot, 20_000);
        assert_eq!(snapshot.health.freshness, RuntimeFreshness::Starting);
        snapshot.settings.paused = true;
        snapshot.health.engine_state = Some(RuntimeEngineState::Paused);
        evaluate_snapshot_health(&mut snapshot, 20_000);
        assert_eq!(snapshot.health.freshness, RuntimeFreshness::Paused);
        snapshot.health.engine_state = Some(RuntimeEngineState::Fatal);
        evaluate_snapshot_health(&mut snapshot, 20_000);
        assert_eq!(snapshot.health.freshness, RuntimeFreshness::Stale);
        assert_eq!(snapshot.sampled_at_ms, None);
        assert_eq!(
            snapshot.health.reason_codes,
            vec![RuntimeHealthReason::EngineFatal]
        );
    }
}
