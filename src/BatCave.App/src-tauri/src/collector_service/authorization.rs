use super::protocol::{
    incompatible, malformed, negotiate_protocol, unauthorized, validate_request, ClientOperationV1,
    ClientRequestV1, ContractFailure, LatestSnapshotRequestV1, ReleaseIdentityV1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifiedPeerAuthorization {
    CollectorClient,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedPeer {
    process_id: u32,
    process_started_at: u64,
    session_id: u32,
    principal_identity: [u8; 32],
    executable_file_identity: [u8; 32],
    executable_release: ReleaseIdentityV1,
    authorization: VerifiedPeerAuthorization,
}

impl VerifiedPeer {
    pub(crate) fn from_transport_verification(
        process_id: u32,
        process_started_at: u64,
        session_id: u32,
        principal_identity: [u8; 32],
        executable_file_identity: [u8; 32],
        executable_release: ReleaseIdentityV1,
        authorization: VerifiedPeerAuthorization,
    ) -> Result<Self, ContractFailure> {
        if process_id == 0
            || process_started_at == 0
            || principal_identity == [0; 32]
            || executable_file_identity == [0; 32]
        {
            return Err(unauthorized("collector_service_peer_identity_invalid"));
        }
        super::protocol::validate_release_identity(&executable_release)?;
        Ok(Self {
            process_id,
            process_started_at,
            session_id,
            principal_identity,
            executable_file_identity,
            executable_release,
            authorization,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerBinding {
    process_id: u32,
    process_started_at: u64,
    session_id: u32,
    principal_identity: [u8; 32],
    executable_file_identity: [u8; 32],
    executable_release: ReleaseIdentityV1,
}

impl From<&VerifiedPeer> for PeerBinding {
    fn from(peer: &VerifiedPeer) -> Self {
        Self {
            process_id: peer.process_id,
            process_started_at: peer.process_started_at,
            session_id: peer.session_id,
            principal_identity: peer.principal_identity,
            executable_file_identity: peer.executable_file_identity,
            executable_release: peer.executable_release.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedServicePeer {
    process_id: u32,
    process_started_at: u64,
    principal_identity: [u8; 32],
    executable_file_identity: [u8; 32],
    executable_release: ReleaseIdentityV1,
}

impl VerifiedServicePeer {
    pub(crate) fn from_transport_verification(
        process_id: u32,
        process_started_at: u64,
        principal_identity: [u8; 32],
        executable_file_identity: [u8; 32],
        executable_release: ReleaseIdentityV1,
    ) -> Result<Self, ContractFailure> {
        if process_id == 0
            || process_started_at == 0
            || principal_identity == [0; 32]
            || executable_file_identity == [0; 32]
        {
            return Err(unauthorized(
                "collector_service_server_peer_identity_invalid",
            ));
        }
        super::protocol::validate_release_identity(&executable_release)?;
        Ok(Self {
            process_id,
            process_started_at,
            principal_identity,
            executable_file_identity,
            executable_release,
        })
    }
}

pub(crate) fn authorize_service_identity(
    peer: Option<&VerifiedServicePeer>,
    claimed: &super::protocol::ServiceIdentityV1,
) -> Result<(), ContractFailure> {
    super::protocol::validate_service_identity(claimed)?;
    let peer = peer.ok_or_else(|| unauthorized("collector_service_server_peer_unverified"))?;
    if peer.executable_release != claimed.release {
        return Err(unauthorized(
            "collector_service_server_release_not_transport_verified",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthorizationState {
    AwaitingNegotiation,
    Active(PeerBinding),
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthorizedOperationV1 {
    Negotiated {
        request_id: u64,
        protocol_version: u16,
    },
    ServiceIdentity {
        request_id: u64,
    },
    LatestSnapshot {
        request_id: u64,
        request: LatestSnapshotRequestV1,
    },
    Ping {
        request_id: u64,
        nonce: u64,
    },
    Disconnect {
        request_id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorizationSession {
    state: AuthorizationState,
}

impl Default for AuthorizationSession {
    fn default() -> Self {
        Self {
            state: AuthorizationState::AwaitingNegotiation,
        }
    }
}

impl AuthorizationSession {
    pub(crate) fn authorize(
        &mut self,
        peer: Option<&VerifiedPeer>,
        request: &ClientRequestV1,
    ) -> Result<AuthorizedOperationV1, ContractFailure> {
        validate_request(request)?;
        let peer = peer.ok_or_else(|| unauthorized("collector_service_peer_unverified"))?;
        if peer.authorization != VerifiedPeerAuthorization::CollectorClient {
            return Err(unauthorized("collector_service_peer_unauthorized"));
        }

        match (&self.state, &request.operation) {
            (AuthorizationState::AwaitingNegotiation, ClientOperationV1::Negotiate(hello)) => {
                if hello.desktop_release != peer.executable_release {
                    return Err(unauthorized(
                        "collector_service_claimed_release_not_transport_verified",
                    ));
                }
                let protocol_version = negotiate_protocol(hello)?;
                self.state = AuthorizationState::Active(peer.into());
                Ok(AuthorizedOperationV1::Negotiated {
                    request_id: request.request_id,
                    protocol_version,
                })
            }
            (AuthorizationState::AwaitingNegotiation, _) => {
                Err(incompatible("collector_service_negotiation_required"))
            }
            (AuthorizationState::Active(_), ClientOperationV1::Negotiate(_)) => {
                Err(malformed("collector_service_already_negotiated"))
            }
            (AuthorizationState::Active(binding), operation) => {
                if binding != &PeerBinding::from(peer) {
                    return Err(unauthorized("collector_service_peer_binding_changed"));
                }
                match operation {
                    ClientOperationV1::ServiceIdentity => {
                        Ok(AuthorizedOperationV1::ServiceIdentity {
                            request_id: request.request_id,
                        })
                    }
                    ClientOperationV1::LatestSnapshot(snapshot_request) => {
                        Ok(AuthorizedOperationV1::LatestSnapshot {
                            request_id: request.request_id,
                            request: snapshot_request.clone(),
                        })
                    }
                    ClientOperationV1::Ping(ping) => Ok(AuthorizedOperationV1::Ping {
                        request_id: request.request_id,
                        nonce: ping.nonce,
                    }),
                    ClientOperationV1::Disconnect => {
                        self.state = AuthorizationState::Disconnected;
                        Ok(AuthorizedOperationV1::Disconnect {
                            request_id: request.request_id,
                        })
                    }
                    ClientOperationV1::Negotiate(_) => {
                        unreachable!("active negotiation handled above")
                    }
                }
            }
            (AuthorizationState::Disconnected, _) => {
                Err(unauthorized("collector_service_session_disconnected"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector_service::protocol::{
        ClientOperationV1, LatestSnapshotRequestV1, NegotiateRequestV1, PingV1,
        ServiceFailureCodeV1, ServiceIdentityV1, ServiceLimitsV1, COLLECTOR_SERVICE_NAME,
        COLLECTOR_SERVICE_PROTOCOL_VERSION,
    };

    #[test]
    fn unverified_and_denied_peers_fail_closed() {
        let request = negotiate_request(release("abc"));
        let mut session = AuthorizationSession::default();

        assert_eq!(
            session.authorize(None, &request).unwrap_err().code,
            ServiceFailureCodeV1::Unauthorized
        );

        let denied = peer(10, 20, release("abc"), VerifiedPeerAuthorization::Denied);
        assert_eq!(
            session.authorize(Some(&denied), &request).unwrap_err().code,
            ServiceFailureCodeV1::Unauthorized
        );
    }

    #[test]
    fn claimed_release_must_match_transport_verified_executable() {
        let request = negotiate_request(release("claimed"));
        let verified = peer(
            10,
            20,
            release("verified"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        let failure = AuthorizationSession::default()
            .authorize(Some(&verified), &request)
            .unwrap_err();
        assert_eq!(failure.code, ServiceFailureCodeV1::Unauthorized);
        assert_eq!(
            failure.detail,
            "collector_service_claimed_release_not_transport_verified"
        );
    }

    #[test]
    fn negotiation_binds_every_followup_to_the_verified_peer() {
        let verified = peer(
            10,
            20,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        let mut session = AuthorizationSession::default();
        assert_eq!(
            session
                .authorize(Some(&verified), &negotiate_request(release("abc")))
                .unwrap(),
            AuthorizedOperationV1::Negotiated {
                request_id: 1,
                protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            }
        );

        let latest = request(
            2,
            ClientOperationV1::LatestSnapshot(LatestSnapshotRequestV1 {
                after_sample_seq: Some(7),
            }),
        );
        assert_eq!(
            session.authorize(Some(&verified), &latest).unwrap(),
            AuthorizedOperationV1::LatestSnapshot {
                request_id: 2,
                request: LatestSnapshotRequestV1 {
                    after_sample_seq: Some(7),
                },
            }
        );

        let other_process = peer(
            11,
            20,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        assert_eq!(
            session
                .authorize(Some(&other_process), &latest)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );
        let other_session = peer(
            10,
            21,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        assert_eq!(
            session
                .authorize(Some(&other_session), &latest)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );
    }

    #[test]
    fn only_negotiation_is_authorized_before_the_session_is_bound() {
        let verified = peer(
            10,
            20,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        for operation in [
            ClientOperationV1::ServiceIdentity,
            ClientOperationV1::LatestSnapshot(LatestSnapshotRequestV1 {
                after_sample_seq: None,
            }),
            ClientOperationV1::Ping(PingV1 { nonce: 4 }),
            ClientOperationV1::Disconnect,
        ] {
            let failure = AuthorizationSession::default()
                .authorize(Some(&verified), &request(2, operation))
                .unwrap_err();
            assert_eq!(failure.code, ServiceFailureCodeV1::Incompatible);
        }
    }

    #[test]
    fn active_session_allows_only_the_frozen_read_only_operations() {
        let verified = peer(
            10,
            20,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        let mut session = active_session(&verified);

        assert_eq!(
            session
                .authorize(
                    Some(&verified),
                    &request(2, ClientOperationV1::ServiceIdentity)
                )
                .unwrap(),
            AuthorizedOperationV1::ServiceIdentity { request_id: 2 }
        );
        assert_eq!(
            session
                .authorize(
                    Some(&verified),
                    &request(3, ClientOperationV1::Ping(PingV1 { nonce: 99 }))
                )
                .unwrap(),
            AuthorizedOperationV1::Ping {
                request_id: 3,
                nonce: 99,
            }
        );

        let second_hello = negotiate_request(release("abc"));
        assert_eq!(
            session
                .authorize(Some(&verified), &second_hello)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Malformed
        );

        assert_eq!(
            session
                .authorize(Some(&verified), &request(4, ClientOperationV1::Disconnect))
                .unwrap(),
            AuthorizedOperationV1::Disconnect { request_id: 4 }
        );
        assert_eq!(
            session
                .authorize(
                    Some(&verified),
                    &request(5, ClientOperationV1::ServiceIdentity)
                )
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );
    }

    #[test]
    fn transport_context_rejects_zero_process_and_malformed_release() {
        assert_eq!(
            VerifiedPeer::from_transport_verification(
                0,
                1,
                1,
                [1; 32],
                [2; 32],
                release("abc"),
                VerifiedPeerAuthorization::CollectorClient,
            )
            .unwrap_err()
            .code,
            ServiceFailureCodeV1::Unauthorized
        );
        assert_eq!(
            VerifiedPeer::from_transport_verification(
                1,
                1,
                1,
                [1; 32],
                [2; 32],
                ReleaseIdentityV1 {
                    app_version: String::new(),
                    source_commit_sha: None,
                },
                VerifiedPeerAuthorization::CollectorClient,
            )
            .unwrap_err()
            .code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn claimed_service_identity_requires_matching_transport_verification() {
        let claimed = service_identity(release("service"));
        assert_eq!(
            authorize_service_identity(None, &claimed).unwrap_err().code,
            ServiceFailureCodeV1::Unauthorized
        );

        let wrong_release = service_peer(release("different"));
        assert_eq!(
            authorize_service_identity(Some(&wrong_release), &claimed)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );

        authorize_service_identity(Some(&service_peer(release("service"))), &claimed).unwrap();
    }

    #[test]
    fn process_reuse_or_file_identity_change_breaks_the_session_binding() {
        let verified = peer(
            10,
            20,
            release("abc"),
            VerifiedPeerAuthorization::CollectorClient,
        );
        let mut session = active_session(&verified);
        let request = request(2, ClientOperationV1::ServiceIdentity);

        let mut reused_process = verified.clone();
        reused_process.process_started_at += 1;
        assert_eq!(
            session
                .authorize(Some(&reused_process), &request)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );

        let mut replaced_file = verified.clone();
        replaced_file.executable_file_identity = [9; 32];
        assert_eq!(
            session
                .authorize(Some(&replaced_file), &request)
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Unauthorized
        );
    }

    fn active_session(peer: &VerifiedPeer) -> AuthorizationSession {
        let mut session = AuthorizationSession::default();
        session
            .authorize(
                Some(peer),
                &negotiate_request(peer.executable_release.clone()),
            )
            .unwrap();
        session
    }

    fn negotiate_request(release: ReleaseIdentityV1) -> ClientRequestV1 {
        ClientRequestV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id: 1,
            operation: ClientOperationV1::Negotiate(NegotiateRequestV1 {
                minimum_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                maximum_protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
                desktop_release: release,
            }),
        }
    }

    fn request(request_id: u64, operation: ClientOperationV1) -> ClientRequestV1 {
        ClientRequestV1 {
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            request_id,
            operation,
        }
    }

    fn peer(
        process_id: u32,
        session_id: u32,
        release: ReleaseIdentityV1,
        authorization: VerifiedPeerAuthorization,
    ) -> VerifiedPeer {
        VerifiedPeer::from_transport_verification(
            process_id,
            1_700_000_000_000,
            session_id,
            [1; 32],
            [2; 32],
            release,
            authorization,
        )
        .unwrap()
    }

    fn service_peer(release: ReleaseIdentityV1) -> VerifiedServicePeer {
        VerifiedServicePeer::from_transport_verification(
            50,
            1_700_000_000_000,
            [3; 32],
            [4; 32],
            release,
        )
        .unwrap()
    }

    fn service_identity(release: ReleaseIdentityV1) -> ServiceIdentityV1 {
        ServiceIdentityV1 {
            service_name: COLLECTOR_SERVICE_NAME.to_string(),
            service_version: "0.2.0-rc.2".to_string(),
            release,
            instance_id: "instance-1".to_string(),
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            minimum_desktop_version: "0.2.0-rc.2".to_string(),
            limits: ServiceLimitsV1::contract(),
        }
    }

    fn release(app_version: &str) -> ReleaseIdentityV1 {
        ReleaseIdentityV1 {
            app_version: app_version.to_string(),
            source_commit_sha: Some("a".repeat(40)),
        }
    }
}
