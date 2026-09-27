use std::time::Duration;

pub(crate) const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const SESSION_MAX_REQUESTS: usize = 4_096;

const RENEW_IDLE_HEADROOM: Duration = Duration::from_secs(5);
const RENEW_REQUEST_HEADROOM: usize = 64;

pub(crate) const SESSION_RENEW_IDLE_AFTER: Duration =
    SESSION_IDLE_TIMEOUT.saturating_sub(RENEW_IDLE_HEADROOM);
pub(crate) const SESSION_RENEW_AFTER_REQUESTS: usize =
    SESSION_MAX_REQUESTS.saturating_sub(RENEW_REQUEST_HEADROOM);

pub(crate) fn should_renew_session(idle_for: Duration, request_count: usize) -> bool {
    idle_for >= SESSION_RENEW_IDLE_AFTER || request_count >= SESSION_RENEW_AFTER_REQUESTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_precedes_the_server_idle_deadline_with_headroom() {
        assert!(SESSION_RENEW_IDLE_AFTER < SESSION_IDLE_TIMEOUT);
        assert!(!should_renew_session(
            SESSION_RENEW_IDLE_AFTER - Duration::from_millis(1),
            0,
        ));
        assert!(should_renew_session(SESSION_RENEW_IDLE_AFTER, 0));
    }

    #[test]
    fn renewal_precedes_the_server_request_cap_with_headroom() {
        assert!(SESSION_RENEW_AFTER_REQUESTS < SESSION_MAX_REQUESTS);
        assert!(!should_renew_session(
            Duration::ZERO,
            SESSION_RENEW_AFTER_REQUESTS - 1,
        ));
        assert!(should_renew_session(
            Duration::ZERO,
            SESSION_RENEW_AFTER_REQUESTS,
        ));
    }
}
