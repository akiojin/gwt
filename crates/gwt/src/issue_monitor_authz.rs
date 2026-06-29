//! SPEC #3200 T-091 — control-plane merge-authorization token.
//!
//! Threat model: `gh pr merge --auto` is SHA-independent, so a `skip_permissions`
//! implementation agent could, in principle, arm an auto-merge that bypasses the
//! strong gate. The defense is that an autonomous merge is only authorized by a
//! token the DAEMON mints — the daemon holds a per-run secret that the partially
//! trusted agents never see. A token binds a merge to a specific
//! `(issue_number, reviewed_sha, base_branch)` tuple, so it cannot be replayed
//! for a different issue or a SHA the strong gate never evaluated.
//!
//! The MAC is HMAC-SHA256 built directly on the `sha2` dependency (avoids an
//! `hmac`/`digest` version split with `sha2 = 0.10`). Verification is
//! constant-time and fail-closed: an empty secret / token / reviewed SHA never
//! verifies, and a tampered field always fails.

use sha2::{Digest, Sha256};

/// SHA-256 block size in bytes.
const BLOCK: usize = 64;

/// Schema-versioned domain separator so a token can never be confused with any
/// other HMAC in the system.
const DOMAIN: &str = "gwt-autonomous-merge/v1";

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    // Normalize the key to one block: hash if longer, zero-pad if shorter.
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        k[..digest.len()].copy_from_slice(&digest);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

fn canonical_message(issue_number: u64, reviewed_sha: &str, base_branch: &str) -> String {
    // Newline-delimited, labeled fields so no concatenation ambiguity can let a
    // different tuple collide with this one.
    format!("{DOMAIN}\nissue={issue_number}\nsha={reviewed_sha}\nbase={base_branch}")
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        s.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap());
    }
    s
}

/// Constant-time byte-slice equality (no early return on first mismatch).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Mint a control-plane token authorizing an autonomous merge of
/// `issue_number` at exactly `reviewed_sha` into `base_branch`. Only call this
/// where the daemon `secret` is available — never expose the secret to agents.
pub fn sign_merge_authorization(
    secret: &[u8],
    issue_number: u64,
    reviewed_sha: &str,
    base_branch: &str,
) -> String {
    let tag = hmac_sha256(
        secret,
        canonical_message(issue_number, reviewed_sha, base_branch).as_bytes(),
    );
    to_hex(&tag)
}

/// Constant-time, fail-closed verification of a merge-authorization token.
/// Returns `false` for an empty secret / token / reviewed SHA, or any tampered
/// field. Only `true` authorizes an autonomous merge.
pub fn verify_merge_authorization(
    secret: &[u8],
    token: &str,
    issue_number: u64,
    reviewed_sha: &str,
    base_branch: &str,
) -> bool {
    if secret.is_empty() || token.is_empty() || reviewed_sha.is_empty() {
        return false;
    }
    let expected = sign_merge_authorization(secret, issue_number, reviewed_sha, base_branch);
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

/// SPEC #3200 layer-4 defense: the SHA that actually merged MUST equal the
/// reviewed SHA the gate authorized. Fail-closed on empty / mismatch.
pub fn merged_sha_matches_reviewed(reviewed_sha: &str, merged_sha: &str) -> bool {
    !reviewed_sha.is_empty() && reviewed_sha == merged_sha
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"daemon-run-secret-abc123";

    #[test]
    fn signing_is_deterministic() {
        let a = sign_merge_authorization(SECRET, 42, "abc123", "main");
        let b = sign_merge_authorization(SECRET, 42, "abc123", "main");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "hex-encoded SHA-256 tag");
    }

    #[test]
    fn a_valid_token_verifies() {
        let token = sign_merge_authorization(SECRET, 42, "abc123", "main");
        assert!(verify_merge_authorization(
            SECRET, &token, 42, "abc123", "main"
        ));
    }

    #[test]
    fn tampering_any_bound_field_fails() {
        let token = sign_merge_authorization(SECRET, 42, "abc123", "main");
        assert!(
            !verify_merge_authorization(SECRET, &token, 99, "abc123", "main"),
            "issue"
        );
        assert!(
            !verify_merge_authorization(SECRET, &token, 42, "def456", "main"),
            "sha"
        );
        assert!(
            !verify_merge_authorization(SECRET, &token, 42, "abc123", "develop"),
            "base"
        );
        assert!(
            !verify_merge_authorization(b"other-secret", &token, 42, "abc123", "main"),
            "secret"
        );
    }

    #[test]
    fn different_secret_yields_different_token() {
        let a = sign_merge_authorization(b"secret-1", 42, "abc123", "main");
        let b = sign_merge_authorization(b"secret-2", 42, "abc123", "main");
        assert_ne!(
            a, b,
            "an agent without the daemon secret cannot forge the token"
        );
    }

    #[test]
    fn empty_inputs_fail_closed() {
        let token = sign_merge_authorization(SECRET, 42, "abc123", "main");
        assert!(!verify_merge_authorization(
            b"", &token, 42, "abc123", "main"
        ));
        assert!(!verify_merge_authorization(
            SECRET, "", 42, "abc123", "main"
        ));
        assert!(!verify_merge_authorization(SECRET, &token, 42, "", "main"));
    }

    #[test]
    fn garbage_token_does_not_verify() {
        assert!(!verify_merge_authorization(
            SECRET,
            "not-a-real-token",
            42,
            "abc123",
            "main"
        ));
        assert!(!verify_merge_authorization(
            SECRET, "deadbeef", 42, "abc123", "main"
        ));
    }

    #[test]
    fn merged_sha_must_equal_reviewed_sha() {
        assert!(merged_sha_matches_reviewed("abc123", "abc123"));
        assert!(
            !merged_sha_matches_reviewed("abc123", "def456"),
            "HEAD advanced"
        );
        assert!(!merged_sha_matches_reviewed("", ""), "empty never matches");
        assert!(!merged_sha_matches_reviewed("abc123", ""));
    }

    #[test]
    fn hmac_matches_known_rfc4231_vector() {
        // RFC 4231 Test Case 2: key="Jefe", data="what do ya want for nothing?".
        let tag = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            to_hex(&tag),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        );
    }
}
