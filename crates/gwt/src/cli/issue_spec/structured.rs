//! CLI input adapter for structured spec editing (SPEC-3060).
//!
//! The structured spec schema, renderer, and merge / split rules moved to
//! [`gwt_github::spec_structured`] so every client shares one owner. This
//! module keeps only the CLI-specific input IO (stdin / file reading) and
//! re-exports the domain API for `cli::issue_spec`.

use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::CliEnv;

#[cfg(test)]
pub(super) use gwt_github::spec_structured::{
    build_user_story_statement, normalize_priority, normalize_user_story_title,
    render_background_section, render_bullet_section, render_numbered_requirement_section,
    split_structured_spec, strip_list_marker, strip_requirement_label, StructuredSpecInput,
    StructuredUserStory, TextBlock,
};
pub(super) use gwt_github::spec_structured::{
    extract_document_title, merge_structured_spec, normalize_spec_heading_from_title,
    parse_structured_spec_json, render_structured_spec,
};

fn io_as_api_error(err: std::io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

/// Read the structured JSON payload from stdin (`-` or omitted) or a file.
pub(super) fn read_cli_input<E: CliEnv>(
    env: &mut E,
    file: Option<&str>,
) -> Result<String, SpecOpsError> {
    match file {
        None | Some("-") => env.read_stdin().map_err(io_as_api_error),
        Some(path) => env.read_file(path).map_err(io_as_api_error),
    }
}
