//! `gwtd issue spec ...` sub-module of the Issue family (SPEC-1942 SC-027 split).
//!
//! Hosts argv `parse` and dispatch `run` for the SPEC subcommand surface
//! plus the family `tests` block. Structured JSON model and render / merge
//! helpers live in [`structured`].

mod structured;

use gwt_github::{
    client::ApiError, Cache, IssueClient, IssueNumber, SectionName, SpecListFilter, SpecOps,
    SpecOpsError,
};

use crate::cli::{CliEnv, CliParseError, ClientRef, IssueCommand};

use std::collections::BTreeMap;

#[cfg(test)]
use structured::{
    build_user_story_statement, normalize_priority, normalize_user_story_title,
    render_background_section, render_bullet_section, render_numbered_requirement_section,
    split_structured_spec, strip_list_marker, strip_requirement_label, StructuredSpecInput,
    StructuredUserStory, TextBlock,
};
use structured::{
    extract_document_title, merge_structured_spec, normalize_spec_heading_from_title,
    parse_structured_spec_json, read_cli_input, render_structured_spec,
};

const SPEC_SECTION_NAME: &str = "spec";
const SPEC_CREATE_HELP: &str = r#"gwtd issue spec create --json --title "SPEC: <short title>" [-f <input.json>]

Structured SPEC input is owned by the CLI. Use this help output as the
single source of truth for the JSON shape and the generated Markdown format.

Input JSON schema:
{
  "background": ["paragraph 1", "paragraph 2"],
  "user_stories": [
    {
      "title": "Short user story title",
      "priority": "P0",
      "status": "IMPLEMENTED",
      "statement": "As a user, I want ..., so that ...",
      "acceptance_scenarios": [
        "Given ..., when ..., then ..."
      ]
    }
  ],
  "edge_cases": ["Edge case text"],
  "functional_requirements": ["Requirement text"],
  "non_functional_requirements": ["Constraint text"],
  "success_criteria": ["Observable outcome text"]
}

Field notes:
- "background": string or array of paragraphs.
- "user_stories[].title": stored as `### US-<n>: <title> (P*)`.
- "user_stories[].priority": `P0`, `P1`, `P2`, or a bare digit.
- "user_stories[].statement": preferred full sentence form. You can also
  provide `as_a`, `i_want`, and `so_that`.
- "functional_requirements": rendered as `FR-001`, `FR-002`, ...
- "non_functional_requirements": rendered as `NFR-001`, `NFR-002`, ...
- "success_criteria": rendered as `SC-001`, `SC-002`, ...

Examples:
gwtd issue spec create --json --title "SPEC: Launch agents from GUI" < spec.json
gwtd issue spec create --json --title "SPEC: Launch agents from GUI" -f spec.json
gwtd issue spec 1942 --edit spec --json -f update.json
gwtd issue spec 1942 --edit spec --json --replace -f replacement.json
"#;

pub(super) fn parse(args: &[&String]) -> Result<IssueCommand, CliParseError> {
    if args.is_empty() {
        return Err(CliParseError::Usage);
    }
    let head = args[0].as_str();
    if head == "list" {
        let mut phase: Option<String> = None;
        let mut state: Option<String> = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--phase" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--phase"));
                    }
                    phase = Some(args[i].clone());
                }
                "--state" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--state"));
                    }
                    state = Some(args[i].clone());
                }
                other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
            }
            i += 1;
        }
        return Ok(IssueCommand::SpecList { phase, state });
    }
    if head == "create" {
        let mut title: Option<String> = None;
        let mut file: Option<String> = None;
        let mut labels: Vec<String> = Vec::new();
        let mut json = false;
        let mut help = false;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--help" => help = true,
                "--json" => json = true,
                "--title" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--title"));
                    }
                    title = Some(args[i].clone());
                }
                "-f" | "--file" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("-f"));
                    }
                    file = Some(args[i].clone());
                }
                "--label" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--label"));
                    }
                    labels.push(args[i].clone());
                }
                other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
            }
            i += 1;
        }
        if help {
            return Ok(IssueCommand::SpecCreateHelp);
        }
        if json {
            return Ok(IssueCommand::SpecCreateJson {
                title: title.ok_or(CliParseError::MissingFlag("--title"))?,
                file,
                labels,
            });
        }
        return Ok(IssueCommand::SpecCreate {
            title: title.ok_or(CliParseError::MissingFlag("--title"))?,
            file: file.ok_or(CliParseError::MissingFlag("-f"))?,
            labels,
        });
    }
    if head == "pull" {
        let mut all = false;
        let mut numbers: Vec<u64> = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--all" => all = true,
                other => {
                    let number = other
                        .parse()
                        .map_err(|_| CliParseError::InvalidNumber(other.to_string()))?;
                    numbers.push(number);
                }
            }
            i += 1;
        }
        return Ok(IssueCommand::SpecPull { all, numbers });
    }
    if head == "repair" {
        if args.len() < 2 {
            return Err(CliParseError::Usage);
        }
        let number = args[1]
            .parse()
            .map_err(|_| CliParseError::InvalidNumber(args[1].clone()))?;
        return Ok(IssueCommand::SpecRepair { number });
    }

    let number = head
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(head.to_string()))?;
    let mut section: Option<String> = None;
    let mut edit_section: Option<String> = None;
    let mut rename_title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut json = false;
    let mut replace = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--section" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--section"));
                }
                section = Some(args[i].clone());
            }
            "--edit" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--edit"));
                }
                edit_section = Some(args[i].clone());
            }
            "--rename" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--rename"));
                }
                rename_title = Some(args[i].clone());
            }
            "--json" => json = true,
            "--replace" => replace = true,
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    if rename_title.is_some() && (section.is_some() || edit_section.is_some() || json || replace) {
        return Err(CliParseError::Usage);
    }
    if let Some(title) = rename_title {
        return Ok(IssueCommand::SpecRename { number, title });
    }
    if let Some(section) = edit_section {
        if json {
            return Ok(IssueCommand::SpecEditSectionJson {
                number,
                section,
                file,
                replace,
            });
        }
        return Ok(IssueCommand::SpecEditSection {
            number,
            section,
            file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        });
    }
    if json || replace {
        return Err(CliParseError::Usage);
    }
    if let Some(section) = section {
        return Ok(IssueCommand::SpecReadSection { number, section });
    }
    Ok(IssueCommand::SpecReadAll { number })
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: IssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match cmd {
        IssueCommand::SpecReadAll { number } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let section_names = [
                "spec",
                "tasks",
                "plan",
                "research",
                "data-model",
                "quickstart",
                "tdd",
            ];
            for name in section_names {
                match ops.read_section(IssueNumber(number), &SectionName(name.to_string())) {
                    Ok(content) => out.push_str(&format!("=== {name} ===\n{content}\n")),
                    Err(SpecOpsError::SectionNotFound(_)) => {}
                    Err(err) => return Err(err),
                }
            }
            0
        }
        IssueCommand::SpecReadSection { number, section } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let content = ops.read_section(IssueNumber(number), &SectionName(section))?;
            out.push_str(&format!("{content}\n"));
            0
        }
        IssueCommand::SpecEditSection {
            number,
            section,
            file,
        } => {
            let content = read_cli_input(env, Some(file.as_str()))?;
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            ops.write_section(IssueNumber(number), &SectionName(section.clone()), &content)?;
            out.push_str(&format!(
                "wrote {} bytes to section '{section}'\n",
                content.len()
            ));
            0
        }
        IssueCommand::SpecEditSectionJson {
            number,
            section,
            file,
            replace,
        } => {
            if section != SPEC_SECTION_NAME {
                return Err(SpecOpsError::from(ApiError::Unexpected(format!(
                    "structured JSON edit only supports section '{SPEC_SECTION_NAME}'"
                ))));
            }
            let raw = read_cli_input(env, file.as_deref())?;
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let structured = parse_structured_spec_json(&raw)?;
            let existing = ops.read_section(IssueNumber(number), &SectionName(section.clone()))?;
            let title = extract_document_title(&existing).unwrap_or_else(|| {
                ops.cache()
                    .load_entry(IssueNumber(number))
                    .map(|entry| normalize_spec_heading_from_title(&entry.snapshot.title))
                    .unwrap_or_else(|| "Specification".to_string())
            });
            let content = if replace {
                render_structured_spec(&title, &structured)
            } else {
                merge_structured_spec(&existing, &structured)
            };
            ops.write_section(IssueNumber(number), &SectionName(section.clone()), &content)?;
            out.push_str(&format!(
                "wrote {} bytes to section '{section}'\n",
                content.len()
            ));
            0
        }
        IssueCommand::SpecList { phase, state } => {
            let filter = SpecListFilter {
                phase,
                state: state.as_deref().and_then(|value| match value {
                    "open" => Some(gwt_github::client::IssueState::Open),
                    "closed" => Some(gwt_github::client::IssueState::Closed),
                    _ => None,
                }),
            };
            let list = env.client().list_spec_issues(&filter)?;
            for spec in list {
                let state_marker = match spec.state {
                    gwt_github::client::IssueState::Open => "OPEN",
                    gwt_github::client::IssueState::Closed => "CLOSED",
                };
                let phase_label = spec
                    .labels
                    .iter()
                    .find(|label: &&String| label.starts_with("phase/"))
                    .cloned()
                    .unwrap_or_default();
                out.push_str(&format!(
                    "#{} [{state_marker}] [{phase_label}] {}\n",
                    spec.number.0, spec.title
                ));
            }
            0
        }
        IssueCommand::SpecCreate {
            title,
            file,
            labels,
        } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let raw = env.read_file(&file).map_err(|err| {
                SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
            })?;
            let parsed = gwt_github::extract_sections(&raw)
                .map_err(|err| SpecOpsError::from(gwt_github::body::ParseError::Section(err)))?;
            let sections: BTreeMap<SectionName, String> = parsed
                .into_iter()
                .map(|section| (section.name, section.content))
                .collect();
            let snapshot = ops.create_spec(&title, sections, &labels)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::SpecCreateJson {
            title,
            file,
            labels,
        } => {
            let raw = read_cli_input(env, file.as_deref())?;
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let structured = parse_structured_spec_json(&raw)?;
            let spec =
                render_structured_spec(&normalize_spec_heading_from_title(&title), &structured);
            let sections = BTreeMap::from([(SectionName(SPEC_SECTION_NAME.to_string()), spec)]);
            let snapshot = ops.create_spec(&title, sections, &labels)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::SpecCreateHelp => {
            out.push_str(SPEC_CREATE_HELP);
            if !SPEC_CREATE_HELP.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        IssueCommand::SpecPull { all, numbers } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            if all {
                let list = env.client().list_spec_issues(&SpecListFilter::default())?;
                for summary in list {
                    ops.read_section(summary.number, &SectionName("spec".to_string()))
                        .ok();
                }
                out.push_str("pulled all gwt-spec issues\n");
            } else if numbers.is_empty() {
                return Err(SpecOpsError::SectionNotFound(
                    "pull requires --all or <n>".into(),
                ));
            } else {
                for number in numbers {
                    ops.read_section(IssueNumber(number), &SectionName("spec".to_string()))?;
                    out.push_str(&format!("pulled #{number}\n"));
                }
            }
            0
        }
        IssueCommand::SpecRepair { number } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            ops.read_section(IssueNumber(number), &SectionName("spec".to_string()))?;
            out.push_str(&format!("repaired cache for #{number}\n"));
            0
        }
        IssueCommand::SpecRename { number, title } => {
            let snapshot = env.client().patch_title(IssueNumber(number), &title)?;
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "renamed issue #{} to '{}'\n",
                snapshot.number.0, title
            ));
            0
        }
        _ => unreachable!("issue_spec::run called with non-spec command"),
    };
    Ok(code)
}
#[cfg(test)]
mod tests;
