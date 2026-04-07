//! Vector index lifecycle for gwt (Phase 8 / SPEC-10 FR-017〜FR-029).
//!
//! This module owns:
//! - On-disk path layout (`paths`)
//! - Manifest read/write for incremental indexing (`manifest`)
//! - File watcher for live re-indexing (`watcher`)
//! - Tokio job spawning for index/refresh/reconcile work (`runtime`)
//!
//! The actual ChromaDB writes happen in the Python runner
//! (`crates/gwt-core/runtime/chroma_index_runner.py`); the Rust side
//! coordinates job dispatch and on-disk metadata only.

pub mod manifest;
pub mod paths;
pub mod runtime;
pub mod watcher;
