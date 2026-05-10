#[test]
fn cli_rs_stays_within_family_split_size_budget() {
    let lines = include_str!("../src/cli.rs").lines().count();
    assert!(
        lines < 1000,
        "SPEC-1942 SC-025 expects crates/gwt/src/cli.rs to stay below 1000 lines; found {lines}"
    );
}
