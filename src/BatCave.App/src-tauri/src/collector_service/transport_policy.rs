use super::{
    authorization::{VerifiedPeer, VerifiedPeerAuthorization},
    host::current_release_identity,
    protocol::{unauthorized, ContractFailure, ReleaseIdentityV1},
};

pub(crate) const DESKTOP_EXECUTABLE_NAME: &str = "batcave-monitor.exe";
// FILE_GENERIC_READ | FILE_WRITE_DATA. FILE_GENERIC_WRITE is deliberately not
// granted because it includes FILE_CREATE_PIPE_INSTANCE for named pipes.
#[cfg(test)]
pub(crate) const PIPE_INTERACTIVE_ACCESS_MASK: u32 = 0x0012_008b;
pub(crate) const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;0x0012008b;;;IU)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutableReleaseEvidence {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub product_version: String,
}

impl ExecutableReleaseEvidence {
    pub(crate) fn package() -> Result<Self, ContractFailure> {
        let core = env!("CARGO_PKG_VERSION")
            .split_once('-')
            .map_or(env!("CARGO_PKG_VERSION"), |(core, _)| core);
        let mut parts = core.split('.');
        let parse = |value: Option<&str>| {
            value
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| unauthorized("collector_service_package_version_invalid"))
        };
        let version = Self {
            major: parse(parts.next())?,
            minor: parse(parts.next())?,
            patch: parse(parts.next())?,
            product_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        if parts.next().is_some() {
            return Err(unauthorized("collector_service_package_version_invalid"));
        }
        Ok(version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedClientEvidence {
    pub process_id: u32,
    pub process_started_at: u64,
    pub pipe_session_id: u32,
    pub process_token_session_id: u32,
    pub impersonated_token_session_id: u32,
    pub process_principal_identity: [u8; 32],
    pub impersonated_principal_identity: [u8; 32],
    pub executable_file_identity: [u8; 32],
    pub process_token_elevated: bool,
    pub impersonated_token_elevated: bool,
    pub executable_path: String,
    pub executable_release: ExecutableReleaseEvidence,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientTrustPolicy {
    service_directory: String,
    expected_release: ReleaseIdentityV1,
    expected_release_evidence: ExecutableReleaseEvidence,
}

impl ClientTrustPolicy {
    pub(crate) fn new(service_directory: &str) -> Result<Self, ContractFailure> {
        let service_directory = normalize_absolute_windows_path(service_directory)
            .ok_or_else(|| unauthorized("collector_service_directory_invalid"))?;
        Ok(Self {
            service_directory,
            expected_release: current_release_identity(),
            expected_release_evidence: ExecutableReleaseEvidence::package()?,
        })
    }

    pub(crate) fn verify(
        &self,
        evidence: VerifiedClientEvidence,
    ) -> Result<VerifiedPeer, ContractFailure> {
        if evidence.process_id == 0
            || evidence.process_started_at == 0
            || evidence.process_principal_identity == [0; 32]
            || evidence.executable_file_identity == [0; 32]
        {
            return Err(unauthorized("collector_service_peer_identity_invalid"));
        }
        if evidence.pipe_session_id != evidence.process_token_session_id
            || evidence.pipe_session_id != evidence.impersonated_token_session_id
        {
            return Err(unauthorized("collector_service_peer_session_mismatch"));
        }
        if evidence.process_principal_identity != evidence.impersonated_principal_identity {
            return Err(unauthorized("collector_service_peer_principal_mismatch"));
        }
        if evidence.process_token_elevated || evidence.impersonated_token_elevated {
            return Err(unauthorized(
                "collector_service_desktop_must_be_standard_user",
            ));
        }
        if evidence.executable_release != self.expected_release_evidence {
            return Err(unauthorized("collector_service_peer_release_mismatch"));
        }
        let executable_path = normalize_absolute_windows_path(&evidence.executable_path)
            .ok_or_else(|| unauthorized("collector_service_peer_path_invalid"))?;
        let (parent, name) = executable_path
            .rsplit_once('\\')
            .ok_or_else(|| unauthorized("collector_service_peer_path_invalid"))?;
        if parent != self.service_directory || !name.eq_ignore_ascii_case(DESKTOP_EXECUTABLE_NAME) {
            return Err(unauthorized(
                "collector_service_peer_executable_unauthorized",
            ));
        }

        VerifiedPeer::from_transport_verification(
            evidence.process_id,
            evidence.process_started_at,
            evidence.pipe_session_id,
            evidence.process_principal_identity,
            evidence.executable_file_identity,
            self.expected_release.clone(),
            VerifiedPeerAuthorization::CollectorClient,
        )
    }
}

fn normalize_absolute_windows_path(value: &str) -> Option<String> {
    let replaced = value.trim().replace('/', "\\");
    let without_prefix = replaced.strip_prefix(r"\\?\").unwrap_or(&replaced);
    let normalized = without_prefix.trim_end_matches('\\').to_ascii_lowercase();
    let bytes = normalized.as_bytes();
    if bytes.len() < 4
        || bytes.get(1) != Some(&b':')
        || bytes.get(2) != Some(&b'\\')
        || normalized
            .split('\\')
            .any(|part| part == "." || part == "..")
    {
        return None;
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_standard_user_in_same_install_root_is_authorized() {
        let policy = ClientTrustPolicy::new(r"C:\Program Files\BatCave Monitor").unwrap();
        policy.verify(evidence()).unwrap();
    }

    #[test]
    fn token_session_principal_elevation_path_and_release_drift_fail_closed() {
        let policy = ClientTrustPolicy::new(r"C:\Program Files\BatCave Monitor").unwrap();

        let mut candidate = evidence();
        candidate.impersonated_token_session_id += 1;
        assert_failure(
            &policy,
            candidate,
            "collector_service_peer_session_mismatch",
        );

        let mut candidate = evidence();
        candidate.impersonated_principal_identity = [9; 32];
        assert_failure(
            &policy,
            candidate,
            "collector_service_peer_principal_mismatch",
        );

        let mut candidate = evidence();
        candidate.process_token_elevated = true;
        assert_failure(
            &policy,
            candidate,
            "collector_service_desktop_must_be_standard_user",
        );

        let mut candidate = evidence();
        candidate.executable_path = r"C:\Users\Public\batcave-monitor.exe".to_string();
        assert_failure(
            &policy,
            candidate,
            "collector_service_peer_executable_unauthorized",
        );

        let mut candidate = evidence();
        candidate.executable_path =
            r"C:\Program Files\BatCave Monitor\..\batcave-monitor.exe".to_string();
        assert_failure(&policy, candidate, "collector_service_peer_path_invalid");

        let mut candidate = evidence();
        candidate.executable_release.patch = candidate.executable_release.patch.saturating_add(1);
        assert_failure(
            &policy,
            candidate,
            "collector_service_peer_release_mismatch",
        );

        let mut candidate = evidence();
        candidate.executable_release.product_version = "0.0.0-different-build".to_string();
        assert_failure(
            &policy,
            candidate,
            "collector_service_peer_release_mismatch",
        );
    }

    #[test]
    fn pipe_acl_names_only_system_and_local_interactive_data_clients() {
        assert!(PIPE_SDDL.contains(";;;SY)"));
        assert!(PIPE_SDDL.contains(";;;IU)"));
        for forbidden in [";;;WD)", ";;;AN)", ";;;NU)", ";;;AU)"] {
            assert!(!PIPE_SDDL.contains(forbidden));
        }

        let interactive_mask = ace_mask(PIPE_SDDL, "IU").expect("interactive ACE exists");
        assert_eq!(interactive_mask, PIPE_INTERACTIVE_ACCESS_MASK);
        assert_ne!(interactive_mask & 0x0000_0001, 0, "FILE_READ_DATA");
        assert_ne!(interactive_mask & 0x0000_0002, 0, "FILE_WRITE_DATA");
        assert_eq!(
            interactive_mask & 0x0000_0004,
            0,
            "FILE_CREATE_PIPE_INSTANCE must never be granted to clients"
        );
    }

    fn ace_mask(sddl: &str, sid: &str) -> Option<u32> {
        sddl.split('(').skip(1).find_map(|ace| {
            let fields = ace.trim_end_matches(')').split(';').collect::<Vec<_>>();
            if fields.get(5).copied() != Some(sid) {
                return None;
            }
            let mask = fields.get(2)?.strip_prefix("0x")?;
            u32::from_str_radix(mask, 16).ok()
        })
    }

    fn evidence() -> VerifiedClientEvidence {
        VerifiedClientEvidence {
            process_id: 41,
            process_started_at: 99,
            pipe_session_id: 2,
            process_token_session_id: 2,
            impersonated_token_session_id: 2,
            process_principal_identity: [1; 32],
            impersonated_principal_identity: [1; 32],
            executable_file_identity: [2; 32],
            process_token_elevated: false,
            impersonated_token_elevated: false,
            executable_path: r"\\?\C:\Program Files\BatCave Monitor\batcave-monitor.exe"
                .to_string(),
            executable_release: ExecutableReleaseEvidence::package().unwrap(),
        }
    }

    fn assert_failure(policy: &ClientTrustPolicy, evidence: VerifiedClientEvidence, detail: &str) {
        assert_eq!(policy.verify(evidence).unwrap_err().detail, detail);
    }
}
