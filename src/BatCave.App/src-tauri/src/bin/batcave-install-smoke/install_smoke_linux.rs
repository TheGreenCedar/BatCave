use super::install_smoke_release::{Failure, SanitizedOutcome, VerifiedLinuxArtifact};

pub(super) fn run(artifact: VerifiedLinuxArtifact) -> Result<SanitizedOutcome, Failure> {
    artifact.revalidate()?;
    Ok(SanitizedOutcome::skipped())
}
