//! Public API contract tests for gwt-skills managed asset distribution.
//!
//! gwt's Start Work / Launch materialization depends on these surfaces:
//! skill bundle distribution into a worktree, stale-asset pruning,
//! gwt-coordination guidance generation, and `.git/info/exclude`
//! management. These tests pin that contract against a throwaway git
//! repository fixture.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gwt_core::process::hidden_command;
use gwt_skills::coordination_guidance::{generate_coordination_guidance, render_skill_md};
use gwt_skills::distribute::distribute_to_worktree;
use gwt_skills::git_exclude::update_git_exclude;

/// Create a real (empty) git repository so asset distribution and
/// `.git/info/exclude` resolution behave as they do in a gwt worktree.
fn init_git_repo(path: &Path) {
    let status = hidden_command("git")
        .args(["init", "--quiet"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init failed for {}", path.display());
}

fn markdown_block<'a>(source: &'a str, start: &str, end: Option<&str>) -> &'a str {
    let start_index = source
        .find(start)
        .unwrap_or_else(|| panic!("missing markdown block start {start:?}"));
    let remainder = &source[start_index..];
    let end_index = end
        .and_then(|marker| remainder.find(marker))
        .unwrap_or(remainder.len());
    &remainder[..end_index]
}

fn line_starting_with<'a>(block: &'a str, prefix: &str) -> &'a str {
    block
        .lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing line starting with {prefix:?}"))
}

fn first_inline_code(line: &str) -> &str {
    line.split('`')
        .nth(1)
        .unwrap_or_else(|| panic!("missing inline code in {line:?}"))
}

#[cfg(unix)]
fn browser_check_shell_block(name: &str) -> String {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let skill_path = workspace_root.join(".claude/skills/browser-check/SKILL.md");
    let skill = fs::read_to_string(&skill_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", skill_path.display()));
    let begin = format!("# browser-check-{name}-begin");
    let end = format!("# browser-check-{name}-end");
    let body = skill
        .split_once(&begin)
        .unwrap_or_else(|| panic!("missing executable browser-check marker {begin}"))
        .1
        .split_once(&end)
        .unwrap_or_else(|| panic!("missing executable browser-check marker {end}"))
        .0;
    body.lines()
        .map(|line| line.strip_prefix("     ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn distribute_to_worktree_materializes_claude_and_codex_skill_bundles() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());

    let report = distribute_to_worktree(dir.path()).expect("distribute bundle");

    assert!(report.files_written > 0, "bundle must write files");
    for skill_md in [
        dir.path().join(".claude/skills/gwt-verify/SKILL.md"),
        dir.path().join(".codex/skills/gwt-verify/SKILL.md"),
    ] {
        assert!(
            skill_md.is_file(),
            "expected bundled skill at {}",
            skill_md.display()
        );
    }

    let has_gwt_command = fs::read_dir(dir.path().join(".claude/commands"))
        .expect("commands dir")
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_name().to_string_lossy().starts_with("gwt-"));
    assert!(
        has_gwt_command,
        "at least one gwt-* command must be written"
    );
}

#[test]
fn repo_keeps_bundled_claude_and_codex_skill_assets_in_parity() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let claude_root = workspace_root.join(".claude/skills");
    let codex_root = workspace_root.join(".codex/skills");

    let claude_files = collect_gwt_skill_files(&claude_root);
    let codex_files = collect_gwt_skill_files(&codex_root);

    assert_eq!(
        claude_files, codex_files,
        "managed gwt-* skill asset file lists must match between .claude and .codex"
    );

    for relative in claude_files {
        let claude = fs::read(claude_root.join(&relative))
            .unwrap_or_else(|err| panic!("read Claude asset {relative:?}: {err}"));
        let codex = fs::read(codex_root.join(&relative))
            .unwrap_or_else(|err| panic!("read Codex asset {relative:?}: {err}"));
        assert!(
            claude == codex,
            "managed gwt-* skill asset must be byte-identical between .claude and .codex: {relative:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn browser_check_authority_script_rejects_local_build_paths_at_any_depth() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let claude_path = workspace_root.join(".claude/skills/browser-check/SKILL.md");
    let codex_path = workspace_root.join(".codex/skills/browser-check/SKILL.md");
    let claude = fs::read_to_string(&claude_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", claude_path.display()));
    let codex = fs::read_to_string(&codex_path)
        .unwrap_or_else(|err| panic!("read {}: {err}", codex_path.display()));

    assert_eq!(
        claude, codex,
        "browser-check must stay byte-identical across Claude and Codex"
    );
    assert!(
        claude.contains("cargo build -p gwt --bin gwt --bin gwtd"),
        "browser-check must build the exact GUI and audit binaries together"
    );

    let script = format!(
        "{}\nfor candidate in \"$@\"; do if is_checkout_local_hook_bin \"$candidate\"; then printf 'reject\\n'; else printf 'allow\\n'; fi; done",
        browser_check_shell_block("hook-authority")
    );
    let output = hidden_command("bash")
        .args([
            "-c",
            &script,
            "browser-check-test",
            "/repo/target/debug/gwtd",
            "/repo/target/aarch64-apple-darwin/debug/gwtd",
            r"C:\repo\TARGET\x86_64-pc-windows-msvc\RELEASE\GWTD.EXE",
            "/Applications/GWT.app/Contents/MacOS/gwtd",
        ])
        .env("GWT_HOOK_BIN", "gwtd")
        .output()
        .expect("run executable authority resolver");
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "reject\nreject\nreject\nallow\n"
    );
}

#[cfg(unix)]
#[test]
fn browser_check_authority_prefers_portable_logical_name_for_stable_path_entry() {
    use std::os::unix::fs::PermissionsExt;

    let tools = tempfile::tempdir().expect("tools tempdir");
    let fake_gwtd = tools.path().join("gwtd");
    fs::write(&fake_gwtd, "#!/bin/sh\nexit 0\n").expect("write fake gwtd");
    fs::set_permissions(&fake_gwtd, fs::Permissions::from_mode(0o755))
        .expect("make fake gwtd executable");
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(tools.path().to_path_buf()).chain(std::env::split_paths(&current_path)),
    )
    .expect("compose PATH");

    let script = format!(
        "{}\nprintf '%s\\n' \"$CHECK_HOOK_BIN\"",
        browser_check_shell_block("hook-authority")
    );
    let output = hidden_command("bash")
        .args(["-c", &script])
        .env_remove("GWT_HOOK_BIN")
        .env("PATH", path)
        .output()
        .expect("run authority resolver with stable PATH entry");

    assert!(output.status.success(), "{:?}", output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "gwtd\n");
}

#[cfg(unix)]
#[test]
fn browser_check_repairs_exact_hook_fallback_before_launch() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let fake_gwtd = dir.path().join("gwtd");
    let capture = dir.path().join("capture.txt");
    fs::write(
        &fake_gwtd,
        "#!/bin/sh\nprintf 'GWT_BIN_PATH=%s\\nGWT_HOOK_BIN=%s\\n' \"${GWT_BIN_PATH-unset}\" \"${GWT_HOOK_BIN-unset}\" > \"$CAPTURE\"\ncat >> \"$CAPTURE\"\nprintf '%s\\n' '{\"ok\":true,\"output\":\"{\\\"repair\\\":{},\\\"health\\\":{\\\"status\\\":\\\"healthy\\\",\\\"issues\\\":[]}}\"}'\n",
    )
    .expect("write fake gwtd");
    fs::set_permissions(&fake_gwtd, fs::Permissions::from_mode(0o755))
        .expect("make fake gwtd executable");

    let output = hidden_command("bash")
        .args(["-c", &browser_check_shell_block("hook-repair")])
        .current_dir(dir.path())
        .env("REPO_ROOT", dir.path())
        .env("CHECK_HOME", dir.path())
        .env("CHECKOUT_GWTD", &fake_gwtd)
        .env("CHECK_HOOK_BIN", "gwtd")
        .env("CAPTURE", &capture)
        .env("GWT_BIN_PATH", "/stale/target/debug/gwtd")
        .output()
        .expect("run hook repair block");

    assert!(
        output.status.success(),
        "hook repair failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(capture).expect("read captured doctor request");
    assert!(captured.contains("GWT_BIN_PATH=unset"), "{captured}");
    assert!(captured.contains("GWT_HOOK_BIN=gwtd"), "{captured}");
    let request = captured.lines().skip(2).collect::<Vec<_>>().join("\n");
    let request: serde_json::Value = serde_json::from_str(&request).expect("doctor request JSON");
    assert_eq!(request["operation"], "hook.doctor");
    assert_eq!(request["params"]["repair"], true);
    assert_eq!(request["params"]["expected_hook_bin"], "gwtd");
}

#[cfg(unix)]
#[test]
fn browser_check_launch_script_unsets_ambient_runtime_override() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let fake_gwt = dir.path().join("target/debug/gwt");
    fs::create_dir_all(fake_gwt.parent().expect("fake gwt parent")).unwrap();
    fs::write(
        &fake_gwt,
        "#!/bin/sh\nprintf '%s\\n' \"${GWT_BIN_PATH-unset}\"\n",
    )
    .unwrap();
    fs::set_permissions(&fake_gwt, fs::Permissions::from_mode(0o755)).unwrap();
    let log = dir.path().join("launch.log");
    let script = format!(
        "ENV_ARGS=(GWT_HOOK_BIN=gwtd)\n{}",
        browser_check_shell_block("launch")
    );
    let output = hidden_command("bash")
        .args(["-c", &script])
        .env("CHECKOUT_GWT", &fake_gwt)
        .env("LOG_FILE", &log)
        .env("GWT_BIN_PATH", "/ambient/stale/gwtd")
        .output()
        .expect("run executable fresh GUI launch block");

    assert!(output.status.success(), "{:?}", output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "unset\n");
    assert_eq!(fs::read_to_string(log).unwrap(), "unset\n");
}

#[test]
fn user_verification_handoff_is_identifiable_and_actionable() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    distribute_to_worktree(dir.path()).expect("distribute bundle");

    for verify_dir in [
        workspace_root.join(".claude/skills/gwt-verify"),
        workspace_root.join(".codex/skills/gwt-verify"),
        dir.path().join(".claude/skills/gwt-verify"),
        dir.path().join(".codex/skills/gwt-verify"),
    ] {
        let skill_path = verify_dir.join("SKILL.md");
        let guide_path = verify_dir.join("references/user-verification-guide.md");
        let skill = fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", skill_path.display()));
        let guide = fs::read_to_string(&guide_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", guide_path.display()));

        for required in [
            "Verification Target Card",
            "Owner Issue/SPEC:",
            "Work purpose:",
            "Success Goal:",
            "Requesting agent/session:",
            "Branch:",
            "Absolute worktree:",
            "Commit:",
            "Prepared instance ID:",
            "URL or launch target:",
            "Action → Expected",
            "Manual Feasibility Gate",
            "Automated-only Evidence",
            "skipped(<reason>)",
        ] {
            assert!(
                skill.contains(required),
                "{} must contain actionable user-verification contract token {required:?}",
                skill_path.display()
            );
            assert!(
                guide.contains(required),
                "{} must contain actionable user-verification contract token {required:?}",
                guide_path.display()
            );
        }

        for (source, path) in [(&skill, &skill_path), (&guide, &guide_path)] {
            let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                normalized.contains("must match a `PASS` entry under `Executed`"),
                "{} must link automated-only evidence to an Executed PASS entry",
                path.display()
            );
        }

        let target_card_fields = [
            "Owner Issue/SPEC:",
            "Work purpose:",
            "Success Goal:",
            "Requesting agent/session:",
            "Branch:",
            "Absolute worktree:",
            "Commit:",
            "Prepared instance ID:",
            "URL or launch target:",
        ];
        let skill_target_card =
            markdown_block(&skill, "#### Verification Target Card", Some("#### 導線"));
        let guide_target_card = markdown_block(
            &guide,
            "## Verification Target Card",
            Some("## The 4-step 導線"),
        );
        for (target_card, path) in [
            (skill_target_card, &skill_path),
            (guide_target_card, &guide_path),
        ] {
            for field in target_card_fields {
                assert!(
                    target_card.contains(field),
                    "{} target card must contain {field:?}",
                    path.display()
                );
            }
        }

        assert_eq!(
            line_starting_with(&skill, "User Verification Result:"),
            "User Verification Result: pending | confirmed | rejected(<reason>) | skipped(<reason>) | n/a",
            "{} evidence-bundle enum must include every supported result",
            skill_path.display()
        );

        for category in ["Expected", "Edge", "Regression"] {
            let actionable_prefix = format!("- [ ] {category} — Action:");
            assert!(
                skill.lines().any(
                    |line| line.starts_with(&actionable_prefix) && line.contains("→ Expected:")
                ),
                "{} must express {category} as Action → Expected",
                skill_path.display()
            );
            assert!(
                guide.lines().any(
                    |line| line.starts_with(&actionable_prefix) && line.contains("→ Expected:")
                ),
                "{} must express {category} as Action → Expected",
                guide_path.display()
            );

            let vague_prefix = format!("- [ ] {category}:");
            assert!(
                !skill.lines().any(|line| line.starts_with(&vague_prefix)),
                "{} must not retain vague {category}-only checkboxes",
                skill_path.display()
            );
            assert!(
                !guide.lines().any(|line| line.starts_with(&vague_prefix)),
                "{} must not retain vague {category}-only checkboxes",
                guide_path.display()
            );

            for (source, path) in [(&skill, &skill_path), (&guide, &guide_path)] {
                for line in source
                    .lines()
                    .filter(|line| line.trim_start().starts_with(&format!("- [ ] {category}")))
                {
                    assert!(
                        line.contains("— Action:") && line.contains("→ Expected:"),
                        "{} has a non-actionable {category} checkbox: {line}",
                        path.display()
                    );
                }
            }
        }

        let skill_target = skill
            .find("#### Verification Target Card")
            .expect("skill target card heading");
        let skill_route = skill[skill_target..]
            .find("#### 導線")
            .map(|offset| skill_target + offset)
            .expect("skill route heading");
        let skill_checks = skill[skill_route..]
            .find("#### Check Items")
            .map(|offset| skill_route + offset)
            .expect("skill check-items heading");
        let skill_automated = skill[skill_checks..]
            .find("#### Automated-only Evidence")
            .map(|offset| skill_checks + offset)
            .expect("skill automated-only evidence heading");
        assert!(
            skill_target < skill_route
                && skill_route < skill_checks
                && skill_checks < skill_automated,
            "{} must present target, route, checks, and automated evidence in order",
            skill_path.display()
        );

        let guide_target = guide
            .find("## Verification Target Card")
            .expect("guide target card heading");
        let guide_route = guide
            .find("## The 4-step 導線")
            .expect("guide route heading");
        let guide_checks = guide.find("## Check Items").expect("guide checks heading");
        let guide_feasibility = guide
            .find("## Manual Feasibility Gate")
            .expect("guide feasibility heading");
        let guide_automated = guide[guide_feasibility..]
            .find("#### Automated-only Evidence")
            .map(|offset| guide_feasibility + offset)
            .expect("guide automated-only evidence heading");
        assert!(
            guide_target < guide_route
                && guide_route < guide_checks
                && guide_checks < guide_feasibility
                && guide_feasibility < guide_automated,
            "{} must define target, route, checks, feasibility, and automated evidence in order",
            guide_path.display()
        );

        let automated_example_command = "cargo test -p gwt --lib \
cli::daemon::server::tests::daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot -- --exact";
        let example_a = markdown_block(&guide, "### Example A", Some("### Example B"));
        let example_b = markdown_block(&guide, "### Example B", Some("### Example C"));
        let example_c = markdown_block(&guide, "### Example C", Some("### Example D"));
        let example_d = markdown_block(&guide, "### Example D", None);

        for (example, label) in [
            (example_a, "Example A"),
            (example_b, "Example B"),
            (example_c, "Example C"),
        ] {
            for field in target_card_fields {
                assert!(
                    example.contains(field),
                    "{} {label} target card must contain {field:?}",
                    guide_path.display()
                );
            }
        }

        let example_a_launch = line_starting_with(example_a, "2. launch:");
        let example_a_navigate = line_starting_with(example_a, "3. navigate:");
        assert!(
            example_a_launch.contains("Prepared instance ID")
                && example_a_launch.contains("do not start another process")
                && !example_a_launch.contains("./target/debug/gwt")
                && !example_a_launch.contains("<port>")
                && example_a_navigate.contains("http://127.0.0.1:61234/")
                && !example_a_navigate.contains("<port>"),
            "{} Example A must route to its exact prepared URL without launching another process",
            guide_path.display()
        );

        let example_b_launch = line_starting_with(example_b, "2. launch:");
        assert!(
            example_b_launch.contains("that exact Editor window")
                && example_b_launch.contains("Assets/Scenes/Main.unity"),
            "{} Example B must stay in its identified prepared editor",
            guide_path.display()
        );

        let example_c_launch = line_starting_with(example_c, "2. launch:");
        assert!(
            example_c_launch.contains("App.exe PID 6840")
                && example_c_launch.contains("do not run another copy")
                && !example_c_launch.contains("dotnet run"),
            "{} Example C must focus its exact prepared process",
            guide_path.display()
        );

        let example_d_executed = line_starting_with(example_d, "- `cargo test");
        let example_d_automated = line_starting_with(example_d, "- Merge-query failure injection:");
        assert_eq!(
            first_inline_code(example_d_executed),
            automated_example_command,
            "{} Example D Executed command must be the expected real test",
            guide_path.display()
        );
        assert_eq!(
            first_inline_code(example_d_executed),
            first_inline_code(example_d_automated),
            "{} Example D automated-only command must exactly match Executed",
            guide_path.display()
        );
        assert!(
            example_d_executed.contains(": PASS")
                && example_d_automated.contains("— PASS")
                && example_d_automated.contains(
                    "daemon_scan_records_merge_reconciliation_error_and_preserves_active_slot"
                ),
            "{} Example D must name the same passing scenario on both sides",
            guide_path.display()
        );
    }
}

#[test]
fn distribute_to_worktree_is_idempotent_for_skill_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let skill_md = dir.path().join(".claude/skills/gwt-verify/SKILL.md");

    distribute_to_worktree(dir.path()).expect("first distribute");
    let first = fs::read_to_string(&skill_md).expect("read after first run");

    distribute_to_worktree(dir.path()).expect("second distribute");
    let second = fs::read_to_string(&skill_md).expect("read after second run");

    assert_eq!(first, second, "re-distribution must be byte-identical");
}

fn collect_gwt_skill_files(skills_root: &Path) -> BTreeSet<PathBuf> {
    let mut files = BTreeSet::new();
    for entry in fs::read_dir(skills_root)
        .unwrap_or_else(|err| panic!("read skills root {}: {err}", skills_root.display()))
    {
        let entry = entry.expect("read skill root entry");
        let file_type = entry.file_type().expect("read skill root entry type");
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // gwt-coordination is generated from coordination_guidance.rs at
        // materialization time and has its own dual-target contract below.
        // This parity check owns only source-controlled bundle assets.
        if file_type.is_dir() && name.starts_with("gwt-") && name != "gwt-coordination" {
            collect_files_relative_to(skills_root, &entry.path(), &mut files);
        }
    }
    files
}

fn collect_files_relative_to(root: &Path, current: &Path, files: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(current)
        .unwrap_or_else(|err| panic!("read asset dir {}: {err}", current.display()))
    {
        let entry = entry.expect("read asset entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read asset entry type");
        if file_type.is_dir() {
            collect_files_relative_to(root, &path, files);
        } else if file_type.is_file() {
            files.insert(
                path.strip_prefix(root)
                    .expect("asset path must be below root")
                    .to_path_buf(),
            );
        }
    }
}

#[test]
fn distribute_to_worktree_prunes_stale_managed_skills() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());

    let stale = dir.path().join(".claude/skills/gwt-retired-test-skill");
    fs::create_dir_all(&stale).expect("create stale skill dir");
    fs::write(stale.join("SKILL.md"), "retired").expect("write stale skill");

    distribute_to_worktree(dir.path()).expect("distribute bundle");

    assert!(
        !stale.exists(),
        "gwt-* skills outside the current bundle must be pruned"
    );
}

#[test]
fn generate_coordination_guidance_writes_skill_for_claude_and_codex() {
    let dir = tempfile::tempdir().expect("tempdir");

    generate_coordination_guidance(dir.path()).expect("generate guidance");

    for skill_md in [
        dir.path().join(".claude/skills/gwt-coordination/SKILL.md"),
        dir.path().join(".codex/skills/gwt-coordination/SKILL.md"),
    ] {
        let content = fs::read_to_string(&skill_md)
            .unwrap_or_else(|e| panic!("read {}: {e}", skill_md.display()));
        assert!(content.contains("gwt-coordination"));
        assert!(
            content.contains("\"operation\":\"board.post\""),
            "guidance must instruct Board posting via gwtd JSON envelopes"
        );
        assert!(
            content.contains(".gwt/work/events/<digest-prefix>/*.jsonl")
                && content.contains("immutable event shard"),
            "generated guidance must deliver new Work events as immutable shards"
        );
        assert!(
            content.contains("frozen read-only compatibility history"),
            "generated guidance must freeze the legacy events.jsonl monolith"
        );
        assert!(
            content.contains(".gwt/work/events/<digest-prefix>/.*.jsonl.create-*"),
            "generated guidance must identify writer temp residue as non-delivery state"
        );
        assert!(
            content.contains("git add -f -- .gwt/work/events/<digest-prefix>/<digest>.jsonl"),
            "generated guidance must explain exact force-add recovery beneath broad ignores"
        );
        assert!(
            content
                .contains("git ls-files --others --ignored --exclude-standard -- .gwt/work/events")
                && content.contains("<2hex>/<64hex>.jsonl"),
            "generated guidance must safely discover every ignored canonical shard"
        );
    }
}

#[test]
fn render_skill_md_embeds_frontmatter_name_and_description() {
    let md = render_skill_md();
    assert!(md.starts_with("---\n"), "must start with YAML frontmatter");
    assert!(md.contains("name: gwt-coordination"));
    assert!(md.contains("\"operation\":\"board.post\""));
}

#[test]
fn update_git_exclude_inserts_managed_block_and_preserves_user_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let exclude_path = dir.path().join(".git/info/exclude");
    fs::create_dir_all(exclude_path.parent().unwrap()).expect("info dir");
    fs::write(&exclude_path, "user-entry.txt\n").expect("seed user entry");

    update_git_exclude(dir.path()).expect("first update");
    update_git_exclude(dir.path()).expect("second update (idempotency)");

    let content = fs::read_to_string(&exclude_path).expect("read exclude");
    assert!(
        content.contains("user-entry.txt"),
        "user entries must be preserved"
    );
    assert_eq!(
        content.matches("# gwt-managed-begin").count(),
        1,
        "managed block must not be duplicated on repeated calls"
    );
    assert_eq!(content.matches("# gwt-managed-end").count(), 1);
}

#[test]
fn git_exclude_tracks_only_canonical_work_history_and_ignores_writer_temp() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_git_repo(dir.path());
    update_git_exclude(dir.path()).expect("update exclude");

    let work = dir.path().join(".gwt/work");
    let events = work.join("events");
    fs::create_dir_all(&events).expect("create events dir");
    fs::write(work.join("events.jsonl"), "legacy\n").expect("write legacy store");
    let bucket = events.join("aa");
    fs::create_dir_all(&bucket).expect("create canonical bucket");
    fs::write(bucket.join(format!("{}.jsonl", "a".repeat(64))), "shard\n")
        .expect("write canonical bucketed shard");
    fs::write(events.join(format!("{}.jsonl", "b".repeat(64))), "flat\n")
        .expect("write flat compatibility shard");
    fs::write(events.join("not-a-hash.jsonl"), "invalid\n").expect("write noncanonical shard");
    fs::write(
        bucket.join(format!(".{}.jsonl.create-123-test", "c".repeat(64))),
        "temp\n",
    )
    .expect("write writer temp");
    fs::write(work.join("memory.md"), "local note\n").expect("write local note");

    let output = hidden_command("git")
        .args(["status", "--short", "--ignored", "--untracked-files=all"])
        .current_dir(dir.path())
        .output()
        .expect("git status");
    assert!(output.status.success(), "git status failed: {output:?}");
    let status = String::from_utf8(output.stdout).expect("utf-8 status");

    assert!(status.contains("?? .gwt/work/events.jsonl\n"), "{status}");
    assert!(
        status.contains(&format!(
            "?? .gwt/work/events/aa/{}.jsonl\n",
            "a".repeat(64)
        )),
        "{status}"
    );
    assert!(
        status.contains(&format!(
            "!! .gwt/work/events/aa/.{}.jsonl.create-123-test\n",
            "c".repeat(64)
        )),
        "{status}"
    );
    assert!(
        status.contains(&format!("!! .gwt/work/events/{}.jsonl\n", "b".repeat(64))),
        "flat compatibility is read-only and must stay ignored for new writes: {status}"
    );
    assert!(
        status.contains("!! .gwt/work/events/not-a-hash.jsonl\n"),
        "{status}"
    );
    assert!(status.contains("!! .gwt/work/memory.md\n"), "{status}");
}

#[test]
fn repository_attributes_leave_immutable_shards_on_default_merge() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let attributes = fs::read_to_string(workspace_root.join(".gitattributes"))
        .expect("read repository .gitattributes");

    assert!(
        attributes.contains("frozen read-only compatibility history"),
        "attributes must document that the legacy union file is frozen"
    );
    assert!(
        attributes.contains("bucketed") && attributes.contains("flat compatibility"),
        "attributes must document that bucketed and compatible flat shards use the default driver"
    );
    assert!(
        attributes
            .lines()
            .any(|line| line == "**/.gwt/work/events.jsonl merge=union"),
        "legacy union behavior must remain compatible"
    );
    assert!(
        !attributes
            .lines()
            .any(|line| { line.contains(".gwt/work/events/") && line.contains("merge=") }),
        "immutable shards must use Git's default merge behavior"
    );
}

#[test]
fn repository_gitignore_tracks_only_bucketed_new_event_shards() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ignore =
        fs::read_to_string(workspace_root.join(".gitignore")).expect("read repository .gitignore");

    assert!(
        ignore
            .lines()
            .any(|line| line == "!.gwt/work/events/[0-9a-f][0-9a-f]/"),
        "canonical two-hex bucket directories must be re-included"
    );
    assert!(
        ignore.lines().any(|line| {
            line.starts_with("!.gwt/work/events/[0-9a-f][0-9a-f]/")
                && line.ends_with(".jsonl")
                && line.matches("[0-9a-f]").count() == 66
        }),
        "canonical bucketed full-digest shard files must be re-included"
    );
    assert!(
        !ignore.lines().any(|line| {
            line.starts_with("!.gwt/work/events/")
                && !line.starts_with("!.gwt/work/events/[0-9a-f][0-9a-f]/")
                && line.ends_with(".jsonl")
        }),
        "W-33 flat shards are read compatibility only and must not be re-included for new writes"
    );
}
