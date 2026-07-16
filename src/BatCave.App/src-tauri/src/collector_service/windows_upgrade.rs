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
            || self.backup_name != upgrade_backup_name(&self.old_digest)
            || !is_staged_upgrade_name(&self.staged_name)
        {
            return Err("collector_service_upgrade_journal_invalid");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpgradeResumeAction {
    ReusePrepared,
    ReusePreparedSameImage,
    CommitCandidate,
    RetryRollback,
    FinalizeVerified,
}

pub(crate) fn staged_transaction_matches(
    journal: &UpgradeJournalV1,
    staged_name: Option<&str>,
    staged_digest: [u8; 32],
) -> bool {
    staged_name == Some(journal.staged_name.as_str()) && staged_digest == journal.new_digest
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
    if backup_digest != journal.old_digest {
        return Err("collector_service_upgrade_journal_identity_invalid");
    }
    match (journal.phase, stable_digest) {
        (UpgradePhase::Prepared, digest)
            if digest == journal.old_digest && journal.old_digest == journal.new_digest =>
        {
            Ok(UpgradeResumeAction::ReusePreparedSameImage)
        }
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
        (UpgradePhase::Verified, _) => Ok(UpgradeResumeAction::RetryRollback),
    }
}

pub(crate) fn upgrade_backup_name(digest: &[u8; 32]) -> String {
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("batcave-collector-service.{digest}.rollback.exe")
}

pub(crate) fn is_staged_upgrade_name(name: &str) -> bool {
    if name == "batcave-collector-service.recovery.exe" {
        return true;
    }
    let Some(version) = name
        .strip_prefix("batcave-collector-service.")
        .and_then(|name| name.strip_suffix(".staged.exe"))
    else {
        return false;
    };
    is_semver(version)
}

fn is_semver(version: &str) -> bool {
    let (without_build, build) = match version.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (version, None),
    };
    if version.matches('+').count() > 1
        || build.is_some_and(|value| !valid_identifiers(value, false))
    {
        return false;
    }
    let (core, prerelease) = match without_build.split_once('-') {
        Some((core, prerelease)) => (core, Some(prerelease)),
        None => (without_build, None),
    };
    if prerelease.is_some_and(|value| !valid_identifiers(value, true)) {
        return false;
    }
    let mut core = core.split('.');
    let Some(major) = core.next() else {
        return false;
    };
    let Some(minor) = core.next() else {
        return false;
    };
    let Some(patch) = core.next() else {
        return false;
    };
    core.next().is_none()
        && [major, minor, patch]
            .into_iter()
            .all(valid_numeric_identifier)
}

fn valid_identifiers(value: &str, reject_numeric_leading_zero: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!reject_numeric_leading_zero
                    || !identifier.bytes().all(|byte| byte.is_ascii_digit())
                    || valid_numeric_identifier(identifier))
        })
}

fn valid_numeric_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(phase: UpgradePhase) -> UpgradeJournalV1 {
        let old_digest = [1; 32];
        UpgradeJournalV1 {
            schema_version: UPGRADE_JOURNAL_SCHEMA_VERSION,
            phase,
            old_digest,
            new_digest: [2; 32],
            backup_name: upgrade_backup_name(&old_digest),
            staged_name: "batcave-collector-service.0.2.0.staged.exe".to_string(),
        }
    }

    #[test]
    fn resume_matrix_is_explicit_and_fail_closed() {
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [1; 32], [2; 32], [1; 32]),
            Ok(UpgradeResumeAction::ReusePrepared)
        );
        let mut same_image = journal(UpgradePhase::Prepared);
        same_image.new_digest = same_image.old_digest;
        assert_eq!(
            decide_upgrade_resume(&same_image, [1; 32], [1; 32], [1; 32]),
            Ok(UpgradeResumeAction::ReusePreparedSameImage)
        );
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [2; 32], [2; 32], [1; 32]),
            Ok(UpgradeResumeAction::CommitCandidate)
        );
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [0; 32], [2; 32], [1; 32]),
            Ok(UpgradeResumeAction::RetryRollback)
        );
        assert_eq!(
            decide_upgrade_resume(
                &journal(UpgradePhase::CandidateInstalled),
                [2; 32],
                [2; 32],
                [1; 32],
            ),
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
            decide_upgrade_resume(&journal(UpgradePhase::Verified), [2; 32], [2; 32], [1; 32],),
            Ok(UpgradeResumeAction::FinalizeVerified)
        );
        assert_eq!(
            decide_upgrade_resume(&journal(UpgradePhase::Verified), [0; 32], [2; 32], [1; 32],),
            Ok(UpgradeResumeAction::RetryRollback)
        );
        assert!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [1; 32], [9; 32], [1; 32],)
                .is_err()
        );
        assert!(
            decide_upgrade_resume(&journal(UpgradePhase::Prepared), [1; 32], [2; 32], [0; 32],)
                .is_err()
        );
    }

    #[test]
    fn journal_rejects_unbound_names_and_zero_identities() {
        for name in [
            "",
            ".",
            "..",
            r"..\escape",
            "C:escape",
            "dir/file",
            "batcave-collector-service.01.rollback.exe",
        ] {
            let mut value = journal(UpgradePhase::Prepared);
            value.backup_name = name.to_string();
            assert!(value.validate().is_err(), "{name}");
        }
        for name in [
            ".",
            "-",
            "1..2",
            "1-",
            "batcave-collector-service..staged.exe",
            "batcave-collector-service.1..2.staged.exe",
            "batcave-collector-service.1-.staged.exe",
        ] {
            let mut value = journal(UpgradePhase::Prepared);
            value.staged_name = name.to_string();
            assert!(value.validate().is_err(), "{name}");
        }
        let mut zero = journal(UpgradePhase::Prepared);
        zero.old_digest = [0; 32];
        assert!(zero.validate().is_err());
    }

    #[test]
    fn staged_names_accept_only_the_fixed_recovery_alias_or_semver() {
        for name in [
            "batcave-collector-service.recovery.exe",
            "batcave-collector-service.0.2.0.staged.exe",
            "batcave-collector-service.0.2.0-rc.2+build.7.staged.exe",
        ] {
            assert!(is_staged_upgrade_name(name), "{name}");
        }
        for name in [
            "batcave-collector-service...staged.exe",
            "batcave-collector-service.-.staged.exe",
            "batcave-collector-service.1..2.staged.exe",
            "batcave-collector-service.1-.staged.exe",
            "batcave-collector-service.01.2.3.staged.exe",
            "batcave-collector-service.1.2.3-01.staged.exe",
        ] {
            assert!(!is_staged_upgrade_name(name), "{name}");
        }
    }

    #[test]
    fn same_name_with_different_bytes_is_a_superseding_transaction() {
        let journal = journal(UpgradePhase::Prepared);
        assert!(staged_transaction_matches(
            &journal,
            Some(journal.staged_name.as_str()),
            journal.new_digest
        ));
        assert!(!staged_transaction_matches(
            &journal,
            Some(journal.staged_name.as_str()),
            [9; 32]
        ));
    }
}
