use std::{env, fs, path::PathBuf};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let skills_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join(".claude")
        .join("skills");

    // Trigger rebuild when skill/command/hook files change.
    println!("cargo:rerun-if-changed=../../.claude/skills");
    println!("cargo:rerun-if-changed=../../.claude/commands");
    println!("cargo:rerun-if-changed=../../.claude/hooks/scripts");

    // Validate YAML frontmatter in all SKILL.md files at build time.
    if skills_dir.is_dir() {
        validate_skill_frontmatter(&skills_dir);
    }
}

/// Validate that every SKILL.md has syntactically correct YAML frontmatter.
///
/// gwt does not interpret skill content at runtime — files are treated as
/// opaque blobs. This validation catches authoring errors at compile time
/// so that malformed skills never reach worktrees.
fn validate_skill_frontmatter(skills_dir: &std::path::Path) {
    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        // Follow symlinks (Codex skills may be symlinked)
        if !ft.is_dir() && !ft.is_symlink() {
            continue;
        }
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        if let Some(frontmatter) = extract_frontmatter(&content) {
            if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(frontmatter) {
                panic!("YAML frontmatter error in {}: {}", skill_md.display(), e);
            }
        }
    }
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
