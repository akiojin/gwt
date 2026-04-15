use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = PathBuf::from(&manifest_dir).join("..").join("..");
    let skills_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join(".claude")
        .join("skills");

    // Trigger rebuild when skill/command/hook files change.
    println!("cargo:rerun-if-changed=../../.claude/skills");
    println!("cargo:rerun-if-changed=../../.claude/commands");

    // Validate YAML frontmatter in all SKILL.md files at build time.
    if skills_dir.is_dir() {
        validate_skill_frontmatter(&workspace_root, &skills_dir);
    }
}

/// Validate that every SKILL.md has syntactically correct YAML frontmatter.
///
/// gwt does not interpret skill content at runtime — files are treated as
/// opaque blobs. This validation catches authoring errors at compile time
/// so that malformed skills never reach worktrees.
fn validate_skill_frontmatter(workspace_root: &std::path::Path, skills_dir: &std::path::Path) {
    for skill_md in tracked_skill_markdowns(workspace_root, skills_dir) {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        if let Some(frontmatter) = extract_frontmatter(&content) {
            if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
                panic!("YAML frontmatter error in {}: {}", skill_md.display(), e);
            }
        }
    }
}

fn tracked_skill_markdowns(
    workspace_root: &std::path::Path,
    skills_dir: &std::path::Path,
) -> Vec<PathBuf> {
    let git_output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .args(["ls-files", "--", ".claude/skills/*/SKILL.md"])
        .output();

    if let Ok(output) = git_output {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(workspace_root.join(trimmed))
                    }
                })
                .collect();
        }
    }

    scanned_skill_markdowns(skills_dir)
}

fn scanned_skill_markdowns(skills_dir: &std::path::Path) -> Vec<PathBuf> {
    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let ft = entry.file_type().ok()?;
            // Follow symlinks (Codex skills may be symlinked)
            if !ft.is_dir() && !ft.is_symlink() {
                return None;
            }
            let skill_md = entry.path().join("SKILL.md");
            skill_md.exists().then_some(skill_md)
        })
        .collect()
}

/// Extract the YAML frontmatter block (between `---` delimiters) from a
/// markdown file.
fn extract_frontmatter(content: &str) -> Option<&str> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end = after_first.find("\n---")?;
    Some(&after_first[..end])
}
