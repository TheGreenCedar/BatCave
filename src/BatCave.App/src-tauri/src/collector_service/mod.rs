pub(crate) mod authorization;
// The lease policy is intentionally present before native ETW wiring. #70 keeps
// the collector service ETW-disabled until persistence and session ownership use
// this decision boundary.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod etw_lease;
pub(crate) mod framing;
pub(crate) mod host;
pub(crate) mod protocol;
pub(crate) mod transport_policy;
#[cfg(windows)]
pub(crate) mod windows_service;
#[cfg(windows)]
pub(crate) mod windows_transport;
