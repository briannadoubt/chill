#![no_main]

use chill_control_plane::{CredentialKind, parse_credential_prefix};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(candidate) = std::str::from_utf8(data) {
        for kind in [
            CredentialKind::SdkKey,
            CredentialKind::UserSession,
            CredentialKind::Service,
        ] {
            let _ = parse_credential_prefix(candidate, kind);
        }
    }
});
