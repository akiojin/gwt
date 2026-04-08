//! gwt-github: GitHub Issue SOT for gwt SPEC management.
//!
//! This crate implements the SPEC-12 design for managing gwt SPECs as GitHub
//! Issues with a hybrid body + comment storage layout and a local cache layer
//! that serves as the source of truth for UI consumers.
//!
//! Modules are introduced incrementally following the SPEC-12 Phase ordering:
//!
//! - [`sections`]: parse/render `<!-- artifact:NAME BEGIN/END -->` markers
//! - [`body`]: assemble/decompose the full [`SpecBody`] from an Issue body and
//!   comment list (to be added)
//! - [`routing`]: decide body vs. comment placement for each section (to be
//!   added)
//!
//! Higher layers (`client`, `cache`, `spec_ops`, `migration`) will be added in
//! subsequent phases.

pub mod body;
pub mod routing;
pub mod sections;

pub use body::{ParseError as BodyParseError, SectionLocation, SectionsIndex, SpecBody, SpecMeta};
pub use routing::{decide_routing, Routing, ROUTING_PROMOTE_THRESHOLD_BYTES};
pub use sections::{
    extract_sections, ExtractedSection, SectionName, SectionParseError, SectionPart,
};
