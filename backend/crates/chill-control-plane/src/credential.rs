use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

const PREFIX_BYTES: usize = 8;
const SECRET_BYTES: usize = 32;
const SECRET_ENCODED_BYTES: usize = 43;
const MINIMUM_PEPPER_BYTES: usize = 32;

/// The credential namespaces accepted by server boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialKind {
    /// Environment-scoped SDK ingestion credential.
    SdkKey,
    /// Internal server worker credential.
    Service,
    /// Human console session.
    UserSession,
}

impl CredentialKind {
    const fn marker(self) -> &'static str {
        match self {
            Self::SdkKey => "ch_sk_",
            Self::Service => "ch_sv_",
            Self::UserSession => "ch_us_",
        }
    }
}

/// A newly issued credential. The raw value must be returned exactly once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Credential {
    /// Full bearer secret shown only at issuance.
    pub raw: String,
    /// Non-secret lookup prefix stored in indexed columns.
    pub prefix: String,
    /// Pepper-bound HMAC-SHA-256 stored instead of plaintext.
    pub digest: [u8; 32],
}

/// Credential parsing and issuance failures.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// The server pepper does not meet the minimum entropy boundary.
    #[error("credential pepper must contain at least {MINIMUM_PEPPER_BYTES} bytes")]
    ShortPepper,
    /// The operating system random source failed.
    #[error("generate credential randomness: {0}")]
    Random(#[from] getrandom::Error),
    /// The bearer value is not canonical for its namespace.
    #[error("malformed credential")]
    Malformed,
}

/// Issues and verifies typed credentials using a process-local secret pepper.
#[derive(Clone)]
pub struct CredentialIssuer {
    pepper: Vec<u8>,
}

impl CredentialIssuer {
    /// Creates an issuer after enforcing the pepper entropy floor.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::ShortPepper`] when fewer than 32 bytes are supplied.
    pub fn new(pepper: &[u8]) -> Result<Self, CredentialError> {
        if pepper.len() < MINIMUM_PEPPER_BYTES {
            return Err(CredentialError::ShortPepper);
        }
        Ok(Self {
            pepper: pepper.to_vec(),
        })
    }

    /// Creates a cryptographically random credential in the requested namespace.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::Random`] when the operating system random source fails.
    pub fn issue(&self, kind: CredentialKind) -> Result<Credential, CredentialError> {
        let mut prefix_bytes = [0_u8; PREFIX_BYTES];
        let mut secret_bytes = [0_u8; SECRET_BYTES];
        getrandom::fill(&mut prefix_bytes)?;
        getrandom::fill(&mut secret_bytes)?;
        let prefix = format!("{}{}", kind.marker(), hex::encode(prefix_bytes));
        let raw = format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(secret_bytes));
        Ok(Credential {
            digest: self.digest(&raw),
            raw,
            prefix,
        })
    }

    /// Compares a bearer value to a stored digest in constant time.
    #[must_use]
    pub fn verify(&self, raw: &str, expected: &[u8]) -> bool {
        expected.len() == 32 && bool::from(self.digest(raw).ct_eq(expected))
    }

    fn digest(&self, raw: &str) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.pepper)
            .unwrap_or_else(|_| unreachable!("HMAC accepts arbitrary key lengths"));
        mac.update(raw.as_bytes());
        mac.finalize().into_bytes().into()
    }
}

/// Extracts the canonical indexed prefix from a typed bearer credential.
///
/// # Errors
///
/// Returns [`CredentialError::Malformed`] for an incorrect namespace, length,
/// alphabet, case, or non-canonical base64url encoding.
pub fn parse_credential_prefix(raw: &str, kind: CredentialKind) -> Result<String, CredentialError> {
    let marker = kind.marker();
    let prefix_length = marker.len() + PREFIX_BYTES * 2;
    if raw.len() != prefix_length + 1 + SECRET_ENCODED_BYTES
        || !raw.starts_with(marker)
        || raw.as_bytes().get(prefix_length) != Some(&b'_')
    {
        return Err(CredentialError::Malformed);
    }
    let prefix_hex = &raw[marker.len()..prefix_length];
    if prefix_hex
        .bytes()
        .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        || hex::decode(prefix_hex).map_or(true, |bytes| bytes.len() != PREFIX_BYTES)
    {
        return Err(CredentialError::Malformed);
    }
    let encoded = &raw[prefix_length + 1..];
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| CredentialError::Malformed)?;
    if decoded.len() != SECRET_BYTES || URL_SAFE_NO_PAD.encode(decoded) != encoded {
        return Err(CredentialError::Malformed);
    }
    Ok(raw[..prefix_length].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_credentials_round_trip_and_remain_disjoint() {
        let issuer = CredentialIssuer::new(&[0x42; 32]).unwrap_or_else(|_| unreachable!());
        let kinds = [
            CredentialKind::SdkKey,
            CredentialKind::Service,
            CredentialKind::UserSession,
        ];
        for kind in kinds {
            let credential = issuer.issue(kind).unwrap_or_else(|_| unreachable!());
            assert_eq!(
                parse_credential_prefix(&credential.raw, kind).ok(),
                Some(credential.prefix.clone())
            );
            assert!(issuer.verify(&credential.raw, &credential.digest));
            assert!(!issuer.verify(&(credential.raw.clone() + "x"), &credential.digest));
            for other in kinds {
                if other != kind {
                    assert!(parse_credential_prefix(&credential.raw, other).is_err());
                }
            }
        }
    }

    #[test]
    fn pepper_and_parser_fail_closed() {
        assert!(CredentialIssuer::new(&[0; 31]).is_err());
        let issuer = CredentialIssuer::new(&[0x11; 32]).unwrap_or_else(|_| unreachable!());
        let other = CredentialIssuer::new(&[0x22; 32]).unwrap_or_else(|_| unreachable!());
        let credential = issuer
            .issue(CredentialKind::SdkKey)
            .unwrap_or_else(|_| unreachable!());
        assert!(!other.verify(&credential.raw, &credential.digest));
        for malformed in [
            "",
            "ch_sk_0011",
            "ch_sk_001122334455667g_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "ch_sk_0011223344556677_AA",
            "CH_sk_0011223344556677_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(parse_credential_prefix(malformed, CredentialKind::SdkKey).is_err());
        }
    }
}
