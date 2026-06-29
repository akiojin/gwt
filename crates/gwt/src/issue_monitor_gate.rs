//! Strong automated merge gate orchestrator for autonomous Issue Monitor mode
//! (SPEC #3200, FR-009 / FR-015 / FR-016 / FR-017).
//!
//! This module composes the three gate elements — (a) real CI success on the
//! reviewed SHA (vacuous green is gate-unavailable), (b) the gwt-verify
//! automated test matrix, and (c) the independent-review verdict — into a
//! single fail-closed `GateOutcome`, signs the privileged
//! `skipped(autonomous-mode)` token bound to the reviewed SHA, and owns the
//! control-plane merge-authorization boundary that closes GitHub's SHA-agnostic
//! `--auto` window.
//!
//! The orchestrator and control-plane token signing/verification land in
//! Phase 2 (T-083 / T-091 / T-092 / T-093). This module is declared here so the
//! Phase 1 foundational data model can reference it and so downstream
//! gate/threat-model tasks can fill it without re-plumbing module wiring.
