use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let skills_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join("plugins")
        .join("gwt")
        .join("skills");
    let commands_dir = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join("plugins")
        .join("gwt")
        .join("commands");

    println!("cargo:rerun-if-changed=../../plugins/gwt/skills");
    println!("cargo:rerun-if-changed=../../plugins/gwt/commands");

    let mut entries = Vec::new();

    if skills_dir.is_dir() {
        let mut skill_dirs: Vec<_> = fs::read_dir(&skills_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
            .collect();
        skill_dirs.sort_by_key(|e| e.file_name());

        for entry in skill_dirs {
            let skill_md = entry.path().join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }

            let content = fs::read_to_string(&skill_md).unwrap();
            let (name, description) = parse_frontmatter(&content);
            if name.is_empty() {
                continue;
            }

            let command_file = commands_dir.join(format!("{name}.md"));
            let has_command = command_file.exists();

            entries.push((name, description, has_command));
        }
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("skill_catalog_generated.rs");

    let mut code = String::new();
    code.push_str("pub struct SkillCatalogEntry {\n");
    code.push_str("    pub name: &'static str,\n");
    code.push_str("    pub description: &'static str,\n");
    code.push_str("    pub has_command: bool,\n");
    code.push_str("}\n\n");
    code.push_str("pub const SKILL_CATALOG: &[SkillCatalogEntry] = &[\n");

    for (name, description, has_command) in &entries {
        let escaped_desc = description.replace('\\', "\\\\").replace('"', "\\\"");
        code.push_str(&format!(
            "    SkillCatalogEntry {{\n        name: \"{name}\",\n        description: \"{escaped_desc}\",\n        has_command: {has_command},\n    }},\n"
        ));
    }

    code.push_str("];\n");

    fs::write(&dest_path, code).unwrap();
}

/// Simple YAML frontmatter parser for `---` delimited blocks.
/// Extracts `name` and `description` fields from key: value lines.
fn parse_frontmatter(content: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();

    let mut lines = content.lines();

    // First line must be `---`
    match lines.next() {
        Some(line) if line.trim() == "---" => {}
        _ => return (name, description),
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }

        // Skip metadata/nested blocks
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            // Strip optional surrounding quotes
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);

            match key {
                "name" => name = value.to_string(),
                "description" => description = value.to_string(),
                _ => {}
            }
        }
    }

    (name, description)
}
