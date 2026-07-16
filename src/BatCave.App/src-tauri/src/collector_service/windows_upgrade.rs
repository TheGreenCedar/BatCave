use serde::{Deserialize, Serialize};

pub(crate) const UPGRADE_JOURNAL_FILE_NAME: &str = "installer-upgrade.v1.json";
pub(crate) const UPGRADE_JOURNAL_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UpgradePhase {
    Prepared,
    CandidateInstalled,
    Verified,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpgradeJournalV1 {
    pub schema_version: u16,
    pub phase: UpgradePhase,
    pub old_digest: [u8; 32],
    pub new_digest: [u8; 32],
    pub backup_name: String,
    pub staged_name: String,
}

impl UpgradeJournalV1 {
    pub(crate) fn new(
        old_digest: [u8; 32],
        new_digest: [u8; 32],
        backup_name: String,
        staged_name: String,
    ) -> Self {
        Self {
            schema_version: UPGRADE_JOURNAL_SCHEMA_VERSION,
            phase: UpgradePhase::Prepared,
            old_digest,
            new_digest,
            backup_name,
            staged_name,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != UPGRADE_JOURNAL_SCHEMA_VERSION
            || self.old_digest == [0; 32]
            || self.new_digest == [0; 32]
            || !is_leaf_name(&self.backup_name)
            || !is_leaf_name(&self.staged_name)
        {
            return Err("collector_service_upgrade_journal_invalid");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpgradeResumeAction {
    ReusePrepared,
    CommitCandidate,
    RetryRollback,
    FinalizeVerified,
}

pub(crate) fn decide_upgrade_resume(
    journal: &UpgradeJournalV1,
    stable_digest: [u8; 32],
    staged_digest: [u8; 32],
    backup_digest: [u8; 32],
) -> Result<UpgradeResumeAction, &'static str> {
    journal.validate()?;
    if staged_digest != journal.new_digest {
        return Err("collector_service_upgrade_journal_identity_invalid");
    }
    if journal.phase != UpgradePhase::Verified && backup_digest != journal.old_digest {
        return Err("collector_service_upgrade_journal_identity_invalid");
    }
    match (journal.phase, stable_digest) {
        (UpgradePhase::Prepared, digest) if digest == journal.old_digest => {
            Ok(UpgradeResumeAction::ReusePrepared)
        }
        (UpgradePhase::Prepared | UpgradePhase::CandidateInstalled, digest)
            if digest == journal.new_digest =>
        {
            Ok(UpgradeResumeAction::CommitCandidate)
        }
        (UpgradePhase::Prepared | UpgradePhase::CandidateInstalled, _) => {
            Ok(UpgradeResumeAction::RetryRollback)
        }
        (UpgradePhase::Verified, digest) if digest == journal.new_digest => {
            Ok(UpgradeResumeAction::FinalizeVerified)
        }
        _ => Err("collector_service_upgrade_journal_state_invalid"),
    }
}

fn is_leaf_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(phase: UpgradePhase) -> UpgradeJournalV1 {
        UpgradeJournalV1 {
            schema_version: UPGRADE_JOURNAL_SCHEMA_VERSION,
            phase,
            old_digest: [1; 32],
            new_digest: [2; 32],
            backup_name: "batcave-collector-service.01.rollback.exe".to_string(),
            staged_name: "batcave-collector-service.0.2.0.staged.exe".to_string(),
        }
    }

    #[test]
    fn resume_matrix_is_explicit_and_fail_closed() {
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [1; 32], [2; 32], [1; 32]),
            Ok(UpgradeResumeAction::ReusePrepared)
        );
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [2; 32], [2; 32], [1; 32]),
            Ok(UpgradeResumeAction::CommitCandidate)
        );
        assert_eq!(
            decide_upgrade_resume(
                &journal(UpgradePhase::CandidateInstalled),
                [9; 32],
                [2; 32],
                [1; 32],
            ),
            Ok(UpgradeResumeAction::RetryRollback)
        );
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Verified), [2; 32], [2; 32], [0; 32],),
            Ok(UpgradeResumeAction::FinalizeVerified)
        );
        assert!(
            decide_upgrade_resume(&journal(UpgradePhase::Verified), [1; 32], [2; 32], [1; 32],)
                .is_err()
        );
        assert!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [1; 32], [9; 32], [1; 32],)
                .is_err()
        );
    }

    #[test]
    fn journal_rejects_paths_and_zero_identities() {
        for name in ["", ".", "..", r"..\escape", "C:escape", "dir/file"] {
            let mut value = journal(UpgradePhase::Prepared);
            value.backup_name = name.to_string();
            assert!(value.validate().is_err(), "{name}");
        }
        let mut zero = journal(UpgradePhase::Prepared);
        zero.old_digest = [0; 32];
        assert!(zero.validate().is_err());
    }
}
