use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Verifies downloaded release binaries against their Sigstore bundles.
///
/// The verifier mirrors Roko's release pipeline: binaries are signed with
/// Sigstore keyless signing from GitHub Actions, and verification requires the
/// expected workflow identity and OIDC issuer to match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigstoreVerifier {
    /// Expected GitHub workflow identity from the signing certificate SAN.
    pub certificate_identity: String,
    /// Expected OIDC issuer for GitHub Actions keyless signing.
    pub certificate_issuer: String,
}

impl SigstoreVerifier {
    /// Create a verifier for Roko release artifacts signed by the release workflow.
    #[must_use]
    pub fn new_for_roko() -> Self {
        Self {
            certificate_identity: format!(
                "https://github.com/nunchi/roko/.github/workflows/release.yml@refs/tags/{}",
                env!("CARGO_PKG_VERSION")
            ),
            certificate_issuer: "https://token.actions.githubusercontent.com".to_string(),
        }
    }

    /// Verify a binary against its Sigstore bundle with `cosign verify-blob`.
    ///
    /// Returns `Ok(())` when the signature is valid, the certificate identity
    /// matches, and the signing event is recorded in the Rekor transparency log.
    ///
    /// # Errors
    ///
    /// Returns an error if `cosign` cannot be executed or if verification fails.
    pub fn verify(&self, binary_path: &Path, bundle_path: &Path) -> Result<()> {
        let status = Command::new("cosign")
            .args(self.verify_blob_args(binary_path, bundle_path))
            .status()
            .context("failed to execute cosign verify-blob")?;

        if !status.success() {
            anyhow::bail!("Sigstore verification failed; binary may be tampered");
        }

        Ok(())
    }

    fn verify_blob_args(&self, binary_path: &Path, bundle_path: &Path) -> Vec<OsString> {
        vec![
            OsString::from("verify-blob"),
            binary_path.as_os_str().to_owned(),
            OsString::from("--bundle"),
            bundle_path.as_os_str().to_owned(),
            OsString::from("--certificate-identity"),
            OsString::from(&self.certificate_identity),
            OsString::from("--certificate-oidc-issuer"),
            OsString::from(&self.certificate_issuer),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn roko_verifier_uses_release_workflow_identity() {
        let verifier = SigstoreVerifier::new_for_roko();

        assert_eq!(
            verifier.certificate_identity,
            format!(
                "https://github.com/nunchi/roko/.github/workflows/release.yml@refs/tags/{}",
                env!("CARGO_PKG_VERSION")
            )
        );
        assert_eq!(
            verifier.certificate_issuer,
            "https://token.actions.githubusercontent.com"
        );
    }

    #[test]
    fn verify_blob_args_match_documented_cosign_invocation() {
        let verifier = SigstoreVerifier::new_for_roko();

        let args = verifier.verify_blob_args(
            Path::new("roko.tar.gz"),
            Path::new("roko.tar.gz.sigstore.json"),
        );

        assert_eq!(
            args,
            vec![
                OsString::from("verify-blob"),
                OsString::from("roko.tar.gz"),
                OsString::from("--bundle"),
                OsString::from("roko.tar.gz.sigstore.json"),
                OsString::from("--certificate-identity"),
                OsString::from(verifier.certificate_identity),
                OsString::from("--certificate-oidc-issuer"),
                OsString::from(verifier.certificate_issuer),
            ]
        );
    }
}
