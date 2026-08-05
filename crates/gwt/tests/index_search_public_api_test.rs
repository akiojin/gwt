use std::path::Path;

use gwt::{
    index_search::{IndexSearchError, ProjectIndexSearchOutcome},
    protocol::{IndexSearchMatchMode, IndexSearchScope},
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
