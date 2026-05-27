use std::{
    fs,
    process::{Command, Stdio},
};

fn run_gwtd_in(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd")
}

#[test]
fn discussion_update_creates_single_canonical_discussions_file() {
    let repo = tempfile::tempdir().expect("repo");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "discussion",
            "update",
            "--date",
            "2026-05-22",
            "--title",
            "Workspace / Work / Discussion terminology",
            "--status",
            "active",
            "--topic",
            "workspace",
            "--topic",
            "work",
            "--related-spec",
            "2359",
            "--summary",
            "Workspace is being split into Project State, Work, Agent, Discussion, and Branch.",
            "--decision",
            "Discussion is not Work.",
            "--decision",
            "Discussions are saved in tasks/discussions.md.",
            "--open-question",
            "How should Topic Stack resume across sessions?",
            "--next",
            "Define Project State migration.",
        ],
    );

    assert!(
        output.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.replace('\\', "/").contains("tasks/discussions.md"),
        "stdout should name updated path, got: {stdout}"
    );
    let content =
        fs::read_to_string(repo.path().join("tasks/discussions.md")).expect("read discussions");
    assert!(content.contains("# Discussions"));
    assert!(content.contains("## 2026-05-22 — Workspace / Work / Discussion terminology"));
    assert!(content.contains("Status: active"));
    assert!(content.contains("Topics: workspace, work"));
    assert!(content.contains("Related SPECs: #2359"));
    assert!(content.contains("- Discussion is not Work."));
    assert!(content.contains("- Discussions are saved in tasks/discussions.md."));
    assert!(content.contains("- How should Topic Stack resume across sessions?"));
    assert!(content.contains("Define Project State migration."));
}

#[test]
fn discussion_update_rewrites_existing_section_instead_of_appending_duplicate() {
    let repo = tempfile::tempdir().expect("repo");

    for summary in ["First summary", "Updated summary"] {
        let output = run_gwtd_in(
            repo.path(),
            &[
                "discussion",
                "update",
                "--date",
                "2026-05-22",
                "--title",
                "Workspace terminology",
                "--status",
                "active",
                "--summary",
                summary,
                "--decision",
                summary,
                "--next",
                "Continue",
            ],
        );
        assert!(
            output.status.success(),
            "discussion update should succeed, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let content =
        fs::read_to_string(repo.path().join("tasks/discussions.md")).expect("read discussions");
    assert_eq!(
        content
            .matches("## 2026-05-22 — Workspace terminology")
            .count(),
        1,
        "active discussion should keep one canonical section"
    );
    assert!(!content.contains("First summary"));
    assert!(content.contains("Updated summary"));
}

#[test]
fn discussion_update_preserves_existing_proposal_blocks() {
    let repo = tempfile::tempdir().expect("repo");

    let first = run_gwtd_in(
        repo.path(),
        &[
            "discussion",
            "update",
            "--date",
            "2026-05-27",
            "--title",
            "Canonical discussion state",
            "--status",
            "active",
            "--summary",
            "Initial summary",
            "--next",
            "Continue",
        ],
    );
    assert!(
        first.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let path = repo.path().join("tasks/discussions.md");
    let mut content = fs::read_to_string(&path).expect("read discussions");
    content.push_str(
        "\n### Proposal A - Store active state in tasks/discussions.md [active]\n\
         - Implementation Proof: pending\n\
         - Next Question: Keep this proposal while refreshing summary?\n",
    );
    fs::write(&path, content).expect("append proposal");

    let second = run_gwtd_in(
        repo.path(),
        &[
            "discussion",
            "update",
            "--date",
            "2026-05-27",
            "--title",
            "Canonical discussion state",
            "--status",
            "active",
            "--summary",
            "Updated summary",
            "--next",
            "Continue again",
        ],
    );
    assert!(
        second.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let updated = fs::read_to_string(path).expect("read discussions");
    assert!(updated.contains("Updated summary"));
    assert!(
        updated.contains("### Proposal A - Store active state in tasks/discussions.md [active]")
    );
    assert!(updated.contains("Keep this proposal while refreshing summary?"));
}
