//! Issue #4449 AC-4 — a mistyped JSON-envelope operation answers with the
//! names it could have meant.
//!
//! `unknown subcommand: workspace.prune` was the entire response an agent got
//! after running the `workspace.prune` a refusal message had recommended. The
//! operation it wanted, `workspace.projection_prune`, was not reachable from
//! anywhere the agent could look; three of them stalled for over an hour on
//! 2026-09-16 before someone read the dispatch source.
//!
//! Whether guidance names operations that exist is Issue #4396's problem, not
//! this suite's.

use gwt::cli::operation_catalog;

/// The near miss that actually happened: a rename, not a typo. No edit
/// distance tight enough to be useful reaches from `workspace.prune` to
/// `workspace.projection_prune`, so segment containment has to carry it.
#[test]
fn suggestions_resolve_the_workspace_prune_near_miss() {
    let suggestions = operation_catalog::suggestions("workspace.prune");
    assert!(
        suggestions.contains(&"workspace.projection_prune"),
        "workspace.prune must suggest workspace.projection_prune, got {suggestions:?}"
    );
}

/// An ordinary typo still resolves through edit distance.
#[test]
fn suggestions_resolve_a_plain_typo() {
    let suggestions = operation_catalog::suggestions("pr.raedy");
    assert!(
        suggestions.contains(&"pr.ready"),
        "a transposition must still resolve, got {suggestions:?}"
    );
}

/// When the name was never a typo of anything — an invented operation rather
/// than a misspelled one — the caller still needs to learn what the namespace
/// does have.
#[test]
fn an_invented_name_falls_back_to_the_namespace_listing() {
    let message = operation_catalog::unknown_subcommand_message("pr.merge");
    assert!(
        message.starts_with("unknown subcommand: pr.merge"),
        "the existing wording must open the message: {message}"
    );
    assert!(
        message.contains("pr.ready") && message.contains("pr.draft"),
        "the message must list the pr. namespace: {message}"
    );
    assert!(
        !message.contains("`pr.merge`"),
        "the unknown name must not be offered back as an option: {message}"
    );
}

/// End to end: the shipped binary itself carries the candidates. The catalog
/// being right is not enough — the agent that lost an hour to
/// `workspace.prune` was reading `gwtd`'s stderr.
#[test]
fn gwtd_answers_a_mistyped_operation_with_candidates() {
    use std::io::Write;
    let mut child = gwt_core::process::hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"schema_version":1,"operation":"workspace.prune","params":{}}"#)
        .expect("write envelope");
    let output = child.wait_with_output().expect("gwtd exits");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown subcommand: workspace.prune"),
        "{stderr}"
    );
    assert!(
        stderr.contains("workspace.projection_prune"),
        "the response must carry the name the caller actually wanted: {stderr}"
    );
}
