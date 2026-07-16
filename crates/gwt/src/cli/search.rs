//! `search` JSON operation family module (SPEC-1942 US-15, FR-106..FR-108).
//!
//! Thin CLI wrapper over [`crate::index_search::search_project_index`] so the
//! semantic search that skills document is a first-class JSON operation instead
//! of a Python-runner-only exception. LLM agents provide scopes through
//! `params.scopes`; the legacy argv parser remains for internal command-model
//! tests. Scope flags mirror
//! [`IndexSearchScope`]; passing no scope flag uses the same default scope
//! merge as the GUI search window.

use gwt_github::{client::ApiError, SpecOpsError};

use crate::protocol::{IndexSearchMatchMode, IndexSearchResult, IndexSearchScope};

use super::{CliEnv, CliParseError};

/// Parsed `search` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCommand {
    pub query: String,
    /// Empty means "GUI default scope merge" (see
    /// `crate::index_search::search_project_index`).
    pub scopes: Vec<IndexSearchScope>,
    pub match_mode: IndexSearchMatchMode,
    pub n_results: Option<usize>,
    pub json: bool,
}

/// Parse a legacy search argv slice into a [`super::CliCommand`]
/// (SPEC-1942 US-15). Lives here instead of `cli.rs` to respect the SC-025
/// family-split size budget, mirroring `register::parse_args`.
pub fn parse_args(args: &[String]) -> Result<super::CliCommand, CliParseError> {
    parse(args).map(super::CliCommand::Search)
}

/// Parse the argv tail after the `search` verb. The query is the single
/// positional argument and may appear before, between, or after flags so the
/// legacy flag-first shape `search --issues "<query>"` parses.
pub fn parse(args: &[String]) -> Result<SearchCommand, CliParseError> {
    let mut query: Option<String> = None;
    let mut scopes: Vec<IndexSearchScope> = Vec::new();
    let mut match_mode = IndexSearchMatchMode::Semantic;
    let mut n_results: Option<usize> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--specs" => push_scope(&mut scopes, IndexSearchScope::Specs),
            "--issues" => push_scope(&mut scopes, IndexSearchScope::Issues),
            "--files" => push_scope(&mut scopes, IndexSearchScope::Files),
            "--files-docs" => push_scope(&mut scopes, IndexSearchScope::FilesDocs),
            "--memory" => push_scope(&mut scopes, IndexSearchScope::Memory),
            "--board" => push_scope(&mut scopes, IndexSearchScope::Board),
            "--discussions" => push_scope(&mut scopes, IndexSearchScope::Discussions),
            "--works" => push_scope(&mut scopes, IndexSearchScope::Works),
            "--json" => json = true,
            "--match-mode" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or(CliParseError::MissingFlag("--match-mode"))?;
                match_mode = match value.as_str() {
                    "semantic" => IndexSearchMatchMode::Semantic,
                    "all_terms" => IndexSearchMatchMode::AllTerms,
                    _ => {
                        return Err(CliParseError::InvalidValue {
                            flag: "--match-mode",
                            reason: "expected semantic or all_terms",
                        })
                    }
                };
            }
            "--n-results" => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or(CliParseError::MissingFlag("--n-results"))?;
                n_results = Some(
                    value
                        .parse()
                        .map_err(|_| CliParseError::InvalidNumber(value.clone()))?,
                );
            }
            other if other.starts_with("--") => {
                return Err(CliParseError::UnknownSubcommand(other.to_string()))
            }
            _ => {
                if query.is_some() {
                    // A second positional argument means an unquoted query;
                    // fail loudly instead of searching a truncated phrase.
                    return Err(CliParseError::Usage);
                }
                query = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let query = query.ok_or(CliParseError::Usage)?;
    if query.trim().is_empty() {
        return Err(CliParseError::Usage);
    }
    Ok(SearchCommand {
        query,
        scopes,
        match_mode,
        n_results,
        json,
    })
}

fn push_scope(scopes: &mut Vec<IndexSearchScope>, scope: IndexSearchScope) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

pub fn run<E: CliEnv>(
    env: &mut E,
    cmd: SearchCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let outcome = match crate::index_search::search_project_index(
        env.repo_path(),
        &cmd.query,
        &cmd.scopes,
        None,
        cmd.match_mode,
        // No watcher exists on the CLI path: the search state machine joins
        // the coordinated repair and waits before failing (Phase 70 FR-388).
        true,
    ) {
        Ok(outcome) => outcome,
        Err(error @ crate::index_search::IndexSearchError::NotReady(_)) => {
            // FR-388: typed retryable failure, never a silent empty success.
            render_not_ready(out, cmd.json, &error);
            return Ok(error.exit_code());
        }
        Err(error) => {
            return Err(SpecOpsError::from(ApiError::Unexpected(error.to_string())));
        }
    };
    let mut results = outcome.results;
    let mut suggestions = outcome.suggestions;
    if let Some(limit) = cmd.n_results {
        results.truncate(limit);
        suggestions.truncate(limit);
    }
    if cmd.json {
        render_json(
            out,
            &cmd.query,
            &results,
            &suggestions,
            &outcome.stale_scopes,
            outcome.refresh_queued,
        );
    } else {
        render_text(out, &results, &suggestions);
        if !outcome.stale_scopes.is_empty() {
            out.push_str(&format!(
                "note: stale scopes [{}] served from the last verified index; refresh queued\n",
                outcome.stale_scopes.join(", ")
            ));
        }
    }
    Ok(0)
}

fn render_not_ready(out: &mut String, json: bool, error: &crate::index_search::IndexSearchError) {
    let crate::index_search::IndexSearchError::NotReady(not_ready) = error else {
        return;
    };
    if json {
        let payload = serde_json::json!({
            "ok": false,
            "error_code": "INDEX_NOT_READY",
            "retryable": true,
            "reason": not_ready.reason,
            "affected_scopes": not_ready.affected_scopes,
            "waited_ms": not_ready.waited_ms,
            "retry_after_ms": not_ready.retry_after_ms,
        });
        out.push_str(&payload.to_string());
        out.push('\n');
    } else {
        out.push_str(&format!("index not ready: {error}\n"));
    }
}

fn render_json(
    out: &mut String,
    query: &str,
    results: &[IndexSearchResult],
    suggestions: &[IndexSearchResult],
    stale_scopes: &[String],
    refresh_queued: bool,
) {
    let mut payload = serde_json::json!({
        "ok": true,
        "query": query,
        "results": results,
        "suggestions": suggestions,
    });
    // Additive freshness metadata (FR-387/FR-398): older clients that do not
    // understand these fields keep processing the legacy success payload.
    if !stale_scopes.is_empty() {
        payload["stale_scopes"] = serde_json::json!(stale_scopes);
        payload["refresh_queued"] = serde_json::json!(refresh_queued);
    }
    out.push_str(&payload.to_string());
    out.push('\n');
}

fn render_text(out: &mut String, results: &[IndexSearchResult], suggestions: &[IndexSearchResult]) {
    if results.is_empty() && suggestions.is_empty() {
        out.push_str("no results\n");
        return;
    }
    for result in results {
        out.push_str(&format_result_line(result));
    }
    if !suggestions.is_empty() {
        out.push_str(&format!(
            "suggestions ({} semantic, not strict matches):\n",
            suggestions.len()
        ));
        for suggestion in suggestions {
            out.push_str(&format_result_line(suggestion));
        }
    }
}

fn format_result_line(result: &IndexSearchResult) -> String {
    let distance = result
        .distance
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "-".to_string());
    if result.subtitle.is_empty() {
        format!(
            "[{}] {} {}\n",
            result.scope.as_str(),
            distance,
            result.title
        )
    } else {
        format!(
            "[{}] {} {} — {}\n",
            result.scope.as_str(),
            distance,
            result.title,
            result.subtitle
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::IndexSearchTarget;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn sample_result(scope: IndexSearchScope, title: &str) -> IndexSearchResult {
        IndexSearchResult {
            scope,
            title: title.to_string(),
            subtitle: "subtitle".to_string(),
            preview: "preview".to_string(),
            distance: Some(0.1234),
            match_mode: None,
            matched_terms: Vec::new(),
            missing_terms: Vec::new(),
            target: IndexSearchTarget::Issue { number: 3049 },
        }
    }

    #[test]
    fn parse_query_only_uses_default_scope_merge() {
        let cmd = parse(&s(&["workspace owner"])).expect("query only must parse");
        assert_eq!(cmd.query, "workspace owner");
        assert!(cmd.scopes.is_empty(), "no flag means default scope merge");
        assert_eq!(cmd.match_mode, IndexSearchMatchMode::Semantic);
        assert_eq!(cmd.n_results, None);
        assert!(!cmd.json);
    }

    #[test]
    fn parse_accepts_query_after_scope_flag() {
        // The exact misuse shape that motivated US-15:
        // Legacy flag-first shape `search --issues "<query>"` must parse.
        let cmd = parse(&s(&["--issues", "workspace owner resume"])).expect("flag-first parse");
        assert_eq!(cmd.query, "workspace owner resume");
        assert_eq!(cmd.scopes, vec![IndexSearchScope::Issues]);
    }

    #[test]
    fn parse_merges_and_dedupes_scope_flags() {
        let cmd = parse(&s(&["--issues", "--specs", "--issues", "q"])).expect("multi scope");
        assert_eq!(
            cmd.scopes,
            vec![IndexSearchScope::Issues, IndexSearchScope::Specs]
        );
    }

    #[test]
    fn parse_supports_every_scope_flag() {
        let cmd = parse(&s(&[
            "q",
            "--specs",
            "--issues",
            "--files",
            "--files-docs",
            "--memory",
            "--board",
            "--discussions",
        ]))
        .expect("all scopes");
        assert_eq!(
            cmd.scopes,
            vec![
                IndexSearchScope::Specs,
                IndexSearchScope::Issues,
                IndexSearchScope::Files,
                IndexSearchScope::FilesDocs,
                IndexSearchScope::Memory,
                IndexSearchScope::Board,
                IndexSearchScope::Discussions,
            ]
        );
    }

    #[test]
    fn parse_match_mode_all_terms() {
        let cmd = parse(&s(&["q", "--match-mode", "all_terms"])).expect("match mode");
        assert_eq!(cmd.match_mode, IndexSearchMatchMode::AllTerms);
    }

    #[test]
    fn parse_match_mode_rejects_unknown_value() {
        let err = parse(&s(&["q", "--match-mode", "fuzzy"])).unwrap_err();
        assert!(
            matches!(
                err,
                CliParseError::InvalidValue {
                    flag: "--match-mode",
                    ..
                }
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn parse_n_results() {
        let cmd = parse(&s(&["q", "--n-results", "5"])).expect("n results");
        assert_eq!(cmd.n_results, Some(5));
    }

    #[test]
    fn parse_n_results_rejects_non_number() {
        let err = parse(&s(&["q", "--n-results", "many"])).unwrap_err();
        assert!(
            matches!(err, CliParseError::InvalidNumber(ref v) if v == "many"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn parse_json_flag() {
        let cmd = parse(&s(&["--json", "q"])).expect("json flag");
        assert!(cmd.json);
    }

    #[test]
    fn parse_missing_query_is_usage() {
        assert!(matches!(parse(&s(&[])), Err(CliParseError::Usage)));
        assert!(matches!(
            parse(&s(&["--issues"])),
            Err(CliParseError::Usage)
        ));
        assert!(matches!(parse(&s(&["   "])), Err(CliParseError::Usage)));
    }

    #[test]
    fn parse_rejects_unknown_flag() {
        let err = parse(&s(&["--bogus", "q"])).unwrap_err();
        assert!(
            matches!(err, CliParseError::UnknownSubcommand(ref v) if v == "--bogus"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn parse_rejects_second_positional_argument() {
        // Unquoted multi-word queries are a usage error, not silent truncation.
        assert!(matches!(
            parse(&s(&["foo", "bar"])),
            Err(CliParseError::Usage)
        ));
    }

    #[test]
    fn render_json_emits_machine_readable_payload() {
        let mut out = String::new();
        render_json(
            &mut out,
            "q",
            &[sample_result(IndexSearchScope::Issues, "issue hit")],
            &[sample_result(IndexSearchScope::Specs, "spec suggestion")],
            &[],
            false,
        );
        let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
        assert_eq!(payload["ok"], serde_json::Value::Bool(true));
        assert_eq!(payload["query"], "q");
        assert_eq!(payload["results"][0]["scope"], "issues");
        assert_eq!(payload["results"][0]["title"], "issue hit");
        assert_eq!(payload["suggestions"][0]["scope"], "specs");
        // No stale metadata unless scopes were actually stale (FR-398
        // additive fields).
        assert!(payload.get("stale_scopes").is_none());
    }

    #[test]
    fn render_json_adds_freshness_metadata_for_stale_scopes() {
        let mut out = String::new();
        render_json(&mut out, "q", &[], &[], &["issues".to_string()], true);
        let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
        assert_eq!(payload["ok"], serde_json::Value::Bool(true));
        assert_eq!(payload["stale_scopes"][0], "issues");
        assert_eq!(payload["refresh_queued"], serde_json::Value::Bool(true));
    }

    #[test]
    fn render_not_ready_json_reports_retryable_error_contract() {
        use crate::index_search::{IndexSearchError, IndexSearchNotReady};
        let mut out = String::new();
        let error = IndexSearchError::NotReady(IndexSearchNotReady {
            reason: "files index is missing".to_string(),
            affected_scopes: vec!["files".to_string()],
            waited_ms: 30_100,
            retry_after_ms: 5_000,
        });
        render_not_ready(&mut out, true, &error);
        let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
        assert_eq!(payload["ok"], serde_json::Value::Bool(false));
        assert_eq!(payload["error_code"], "INDEX_NOT_READY");
        assert_eq!(payload["retryable"], serde_json::Value::Bool(true));
        assert_eq!(payload["affected_scopes"][0], "files");
        assert_eq!(payload["waited_ms"], 30_100);
        assert_eq!(payload["retry_after_ms"], 5_000);
        assert_eq!(error.exit_code(), 75);
    }

    #[test]
    fn render_text_lists_results_and_suggestions() {
        let mut out = String::new();
        render_text(
            &mut out,
            &[sample_result(IndexSearchScope::Issues, "issue hit")],
            &[sample_result(IndexSearchScope::Specs, "spec suggestion")],
        );
        assert!(out.contains("[issues]"), "scope tag missing: {out}");
        assert!(out.contains("issue hit"), "result title missing: {out}");
        assert!(
            out.contains("suggestions"),
            "suggestion header missing: {out}"
        );
        assert!(out.contains("spec suggestion"), "suggestion missing: {out}");
    }

    #[test]
    fn render_text_reports_empty_results() {
        let mut out = String::new();
        render_text(&mut out, &[], &[]);
        assert!(out.contains("no results"), "empty notice missing: {out}");
    }
}
