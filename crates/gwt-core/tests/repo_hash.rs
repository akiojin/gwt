//! Phase 8: integration tests for `gwt_core::repo_hash`.
//!
//! These tests will fail to compile until `crates/gwt-core/src/repo_hash.rs`
//! exists and is exported from `lib.rs`.

use gwt_core::repo_hash::{compute_repo_hash, normalize_origin_url, RepoHash};

#[test]
fn normalize_https_url_strips_dot_git_and_lowercases() {
    assert_eq!(
        normalize_origin_url("https://github.com/Akiojin/gwt.git"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn normalize_ssh_url_matches_https() {
    let https = normalize_origin_url("https://github.com/akiojin/gwt.git");
    let ssh = normalize_origin_url("git@github.com:akiojin/gwt.git");
    assert_eq!(https, ssh);
}

#[test]
fn normalize_ssh_with_protocol_form() {
    assert_eq!(
        normalize_origin_url("ssh://git@github.com:22/akiojin/gwt.git"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn normalize_handles_trailing_slash() {
    assert_eq!(
        normalize_origin_url("https://github.com/akiojin/gwt/"),
        "github.com/akiojin/gwt"
    );
}

#[test]
fn compute_repo_hash_returns_16_lowercase_hex_chars() {
    let h = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let hex = h.as_str();
    assert_eq!(hex.len(), 16);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hash must be lowercase hex: {hex}"
    );
}

#[test]
fn compute_repo_hash_is_deterministic() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn https_and_ssh_forms_yield_same_hash() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("git@github.com:akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn different_repos_yield_different_hashes() {
    let a = compute_repo_hash("https://github.com/akiojin/gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/other.git");
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn case_insensitive_path_yields_same_hash() {
    let a = compute_repo_hash("https://GitHub.com/Akiojin/Gwt.git");
    let b = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(a.as_str(), b.as_str());
}

#[test]
fn repo_hash_display_equals_as_str() {
    let h: RepoHash = compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(format!("{h}"), h.as_str());
}
