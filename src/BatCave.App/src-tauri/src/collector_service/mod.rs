pub(crate) mod authorization;
pub(crate) mod client;
// The collector service starts ETW only through this fail-closed lease policy;
// crash reclaim remains deferred until the complete recovery path is proven.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod etw_lease;
pub(crate) mod framing;
pub(crate) mod host;
#[cfg(any(windows, test))]
pub(crate) mod listener_lifecycle;
pub(crate) mod protocol;
pub(crate) mod session_lease;
pub(crate) mod transport_policy;
#[cfg(windows)]
pub(crate) mod windows_client;
#[cfg(windows)]
pub(crate) mod windows_provisioner;
#[cfg(windows)]
pub(crate) mod windows_service;
#[cfg(windows)]
pub(crate) mod windows_shortcut_retirement;
#[cfg(windows)]
pub(crate) mod windows_transport;
#[cfg(windows)]
pub(crate) mod windows_upgrade;
