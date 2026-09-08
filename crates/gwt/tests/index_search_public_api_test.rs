use std::path::Path;

use gwt::{
    index_search::{IndexSearchError, ProjectIndexSearchOutcome},
    protocol::{IndexSearchMatchMode, IndexSearchResult, IndexSearchScope, IndexSearchTarget},
};

type PublicSearchFn = fn(
    &Path,
    &str,
    &[IndexSearchScope],
    Option<&str>,
    IndexSearchMatchMode,
    bool,
) -> Result<ProjectIndexSearchOutcome, IndexSearchError>;

// This is intentionally an exhaustive downstream-crate match. Adding a
// public variant to IndexSearchError is a source-breaking API change and must
// fail this compile-time contract.
fn classify_public_error(error: IndexSearchError) -> &'static str {
    match error {
        IndexSearchError::NotReady(_) => "not-ready",
        IndexSearchError::SearchFailed(_) => "search-failed",
        IndexSearchError::Other(_) => "other",
    }
}

#[test]
fn public_index_search_error_remains_exhaustive_with_three_variants() {
    let _search: PublicSearchFn = gwt::search_project_index;

    assert_eq!(
        classify_public_error(IndexSearchError::Other("legacy".to_string())),
        "other"
    );
}

#[test]
fn public_file_search_result_shape_remains_exhaustive() {
    let result = IndexSearchResult {
        scope: IndexSearchScope::Files,
        title: "src/lib.rs".to_string(),
        subtitle: "rs".to_string(),
        preview: "library entrypoint".to_string(),
        distance: Some(0.1234),
        match_mode: Some(IndexSearchMatchMode::AllTerms),
        matched_terms: vec!["library".to_string()],
        missing_terms: Vec::new(),
        target: IndexSearchTarget::File {
            path: "src/lib.rs".to_string(),
        },
    };
    let IndexSearchResult {
        scope,
        title,
        subtitle,
        preview,
        distance,
        match_mode,
        matched_terms,
        missing_terms,
        target,
    } = result;
    assert_eq!(scope, IndexSearchScope::Files);
    assert_eq!(title, "src/lib.rs");
    assert_eq!(subtitle, "rs");
    assert_eq!(preview, "library entrypoint");
    assert_eq!(distance, Some(0.1234));
    assert_eq!(match_mode, Some(IndexSearchMatchMode::AllTerms));
    assert_eq!(matched_terms, ["library"]);
    assert!(missing_terms.is_empty());
    assert_eq!(
        target,
        IndexSearchTarget::File {
            path: "src/lib.rs".to_string()
        }
    );

    let outcome = ProjectIndexSearchOutcome {
        results: Vec::new(),
        suggestions: Vec::new(),
        stale_scopes: Vec::new(),
        refresh_queued: false,
    };
    let ProjectIndexSearchOutcome {
        results,
        suggestions,
        stale_scopes,
        refresh_queued,
    } = outcome;
    assert!(results.is_empty());
    assert!(suggestions.is_empty());
    assert!(stale_scopes.is_empty());
    assert!(!refresh_queued);
}
