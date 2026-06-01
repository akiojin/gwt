//! Remote Board providers (Slack / Teams) — SPEC-2963.
//!
//! When `board.provider` is `slack` or `teams`, the Board's source of truth is
//! the remote SaaS (remote-sole). This module hosts the OAuth flow, secure
//! token storage, response caching, field mapping, and the provider
//! implementations of [`gwt_core::coordination::BoardProvider`].

pub mod token_store;
