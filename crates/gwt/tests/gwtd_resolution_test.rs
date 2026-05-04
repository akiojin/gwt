use std::{collections::HashSet, path::PathBuf};

use gwt::cli::gwtd_resolver::{resolve_gwtd_path_with, GwtdResolutionInputs};

fn inputs(
    existing: impl IntoIterator<Item = PathBuf>,
    path_candidate: Option<PathBuf>,
) -> GwtdResolutionInputs<'static> {
    let existing = existing.into_iter().collect::<HashSet<_>>();
    GwtdResolutionInputs {
        explicit_bin_path: None,
        path_lookup: Box::new(move |command| {
            assert_eq!(command, "gwtd");
            path_candidate.clone()
        }),
        installed_candidates: vec![PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")],
        development_fallbacks: vec![PathBuf::from("/repo/target/debug/gwtd")],
        is_file: Box::new(move |path| existing.contains(path)),
    }
}

#[test]
fn explicit_gwt_bin_path_wins_over_path_and_fallbacks() {
    let mut inputs = inputs(
        [
            PathBuf::from("/explicit/gwtd"),
            PathBuf::from("/path/gwtd"),
            PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd"),
            PathBuf::from("/repo/target/debug/gwtd"),
        ],
        Some(PathBuf::from("/path/gwtd")),
    );
    inputs.explicit_bin_path = Some(PathBuf::from("/explicit/gwtd"));

    let resolved = resolve_gwtd_path_with(inputs).expect("resolve explicit gwtd");

    assert_eq!(resolved, PathBuf::from("/explicit/gwtd"));
}

#[test]
fn path_gwtd_wins_when_explicit_path_is_missing() {
    let resolved = resolve_gwtd_path_with(inputs(
        [
            PathBuf::from("/path/gwtd"),
            PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd"),
            PathBuf::from("/repo/target/debug/gwtd"),
        ],
        Some(PathBuf::from("/path/gwtd")),
    ))
    .expect("resolve PATH gwtd");

    assert_eq!(resolved, PathBuf::from("/path/gwtd"));
}

#[test]
fn macos_app_bundle_wins_when_path_is_missing() {
    let resolved = resolve_gwtd_path_with(inputs(
        [
            PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd"),
            PathBuf::from("/repo/target/debug/gwtd"),
        ],
        None,
    ))
    .expect("resolve app bundle gwtd");

    assert_eq!(
        resolved,
        PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")
    );
}

#[test]
fn repo_local_development_binary_is_last_fallback() {
    let resolved = resolve_gwtd_path_with(inputs([PathBuf::from("/repo/target/debug/gwtd")], None))
        .expect("resolve repo-local fallback gwtd");

    assert_eq!(resolved, PathBuf::from("/repo/target/debug/gwtd"));
}

#[test]
fn missing_all_candidates_returns_none() {
    let resolved = resolve_gwtd_path_with(inputs([], None));

    assert!(resolved.is_none());
}
