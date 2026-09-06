//! Versioned local screening. This module has no network client or online fallback.
use crate::contract::{IdentityError, PasswordScreen};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read, path::Path};

pub const COMMON_PASSWORD_VERSION: &str = "bbbcm-common-v1";
// A deliberately small, versioned rejection list, never presented as a complete
// breach corpus. Comparison catches ASCII case variants without changing hashes.
const COMMON_PASSWORDS: &[&str] = &[
    "password",
    "password1",
    "password123",
    "password1234",
    "password!",
    "password1!",
    "12345678",
    "123456789",
    "1234567890",
    "0123456789",
    "11111111",
    "00000000",
    "88888888",
    "66666666",
    "87654321",
    "qwertyui",
    "qwertyuiop",
    "qwerty123",
    "qwerty1234",
    "asdfghjk",
    "asdfghjkl",
    "zxcvbnm123",
    "iloveyou",
    "letmein123",
    "welcome1",
    "welcome123",
    "admin123",
    "admin1234",
    "administrator",
    "changeme",
    "abc12345",
    "abc123456",
    "1qaz2wsx",
    "1q2w3e4r",
    "starwars",
    "monkey123",
];

#[derive(Default)]
pub struct CommonPasswordScreen;
impl CommonPasswordScreen {
    pub fn new() -> Self {
        Self
    }
    pub fn version(&self) -> &'static str {
        COMMON_PASSWORD_VERSION
    }
}
impl PasswordScreen for CommonPasswordScreen {
    fn allowed(&self, password: &str) -> Result<bool, IdentityError> {
        Ok(!COMMON_PASSWORDS
            .iter()
            .any(|weak| weak.eq_ignore_ascii_case(password)))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    corpus_version: String,
    algorithm: String,
    sha256_digests: Vec<String>,
}

/// An immutable startup-validated corpus. Missing, invalid or mismatched files
/// return Unavailable; callers must not fall back to the common-only screen.
pub struct OfflinePasswordScreen {
    version: String,
    hashes: BTreeSet<String>,
}
impl OfflinePasswordScreen {
    pub fn load(path: &Path, expected_sha256: &str) -> Result<Self, IdentityError> {
        const MAX_BYTES: u64 = 64 * 1024 * 1024;
        if expected_sha256.len() != 64 || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(IdentityError::Unavailable);
        }
        let metadata = std::fs::symlink_metadata(path).map_err(|_| IdentityError::Unavailable)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
            return Err(IdentityError::Unavailable);
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| IdentityError::Unavailable)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| IdentityError::Unavailable)?;
        if bytes.len() as u64 > MAX_BYTES
            || !crate::core::constant_equal(
                &format!("{:x}", Sha256::digest(&bytes)),
                &expected_sha256.to_ascii_lowercase(),
            )
        {
            return Err(IdentityError::Unavailable);
        }
        let corpus: Corpus =
            serde_json::from_slice(&bytes).map_err(|_| IdentityError::Unavailable)?;
        if corpus.schema_version != 1
            || corpus.algorithm != "sha256"
            || corpus.corpus_version.is_empty()
            || corpus.corpus_version.len() > 64
            || !corpus
                .corpus_version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            || corpus.sha256_digests.is_empty()
            || corpus
                .sha256_digests
                .iter()
                .any(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(IdentityError::Unavailable);
        }
        Ok(Self {
            version: corpus.corpus_version,
            hashes: corpus
                .sha256_digests
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
        })
    }
    pub fn version(&self) -> &str {
        &self.version
    }
}
impl PasswordScreen for OfflinePasswordScreen {
    fn allowed(&self, password: &str) -> Result<bool, IdentityError> {
        Ok(CommonPasswordScreen.allowed(password)?
            && !self
                .hashes
                .contains(&format!("{:x}", Sha256::digest(password.as_bytes()))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_and_versioned_offline_corpus_fail_closed() {
        let dir = std::env::temp_dir().join(format!(
            "bbbcm-password-corpus-{}",
            platform::transaction::new_operation_id()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("synthetic.json");
        let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"corpus_version":"synthetic-v1","algorithm":"sha256","sha256_digests":[format!("{:x}",Sha256::digest(b"synthetic-compromised-value"))]})).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        std::fs::write(&path, &bytes).unwrap();
        let screen = OfflinePasswordScreen::load(&path, &digest).unwrap();
        assert_eq!(screen.version(), "synthetic-v1");
        assert!(!screen.allowed("synthetic-compromised-value").unwrap());
        assert!(!screen.allowed("PassWord123").unwrap());
        assert!(
            screen
                .allowed("synthetic-independent-strong-value")
                .unwrap()
        );
        assert!(OfflinePasswordScreen::load(&path, &"0".repeat(64)).is_err());
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(OfflinePasswordScreen::load(&path, &digest).is_err());
        let corrupt_digest = format!("{:x}", Sha256::digest(b"corrupt"));
        assert!(OfflinePasswordScreen::load(&path, &corrupt_digest).is_err());
        std::fs::remove_file(&path).unwrap();
        assert!(OfflinePasswordScreen::load(&path, &digest).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
