use std::collections::BTreeMap;

use gwt_github::{
    client::ApiError, Cache, IssueClient, IssueNumber, SectionName, SpecListFilter, SpecOps,
    SpecOpsError,
};
use serde::Deserialize;

use crate::cli::{CliCommand, CliEnv, CliParseError, ClientRef};

const SPEC_SECTION_NAME: &str = "spec";
const CANONICAL_SECTION_HEADINGS: [&str; 6] = [
    "Background",
    "User Stories",
    "Edge Cases",
    "Functional Requirements",
    "Non-Functional Requirements",
    "Success Criteria",
];
const SPEC_CREATE_HELP: &str = r#"gwt issue spec create --json --title "SPEC: <short title>" [-f <input.json>]

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
gwt issue spec create --json --title "SPEC: Launch agents from GUI" < spec.json
gwt issue spec create --json --title "SPEC: Launch agents from GUI" -f spec.json
gwt issue spec 1942 --edit spec --json -f update.json
gwt issue spec 1942 --edit spec --json --replace -f replacement.json
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TextBlock {
    Text(String),
    Paragraphs(Vec<String>),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct StructuredSpecInput {
    #[serde(default)]
    background: Option<TextBlock>,
    #[serde(default)]
    user_stories: Option<Vec<StructuredUserStory>>,
    #[serde(default)]
    edge_cases: Option<Vec<String>>,
    #[serde(default)]
    functional_requirements: Option<Vec<String>>,
    #[serde(default)]
    non_functional_requirements: Option<Vec<String>>,
    #[serde(default)]
    success_criteria: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
struct StructuredUserStory {
    title: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    statement: Option<String>,
    #[serde(default)]
    as_a: Option<String>,
    #[serde(default)]
    i_want: Option<String>,
    #[serde(default)]
    so_that: Option<String>,
    #[serde(default, alias = "acceptance")]
    acceptance_scenarios: Vec<String>,
}

pub(super) fn parse(args: &[&String]) -> Result<CliCommand, CliParseError> {
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
        return Ok(CliCommand::SpecList { phase, state });
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
            return Ok(CliCommand::SpecCreateHelp);
        }
        if json {
            return Ok(CliCommand::SpecCreateJson {
                title: title.ok_or(CliParseError::MissingFlag("--title"))?,
                file,
                labels,
            });
        }
        return Ok(CliCommand::SpecCreate {
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
        return Ok(CliCommand::SpecPull { all, numbers });
    }
    if head == "repair" {
        if args.len() < 2 {
            return Err(CliParseError::Usage);
        }
        let number = args[1]
            .parse()
            .map_err(|_| CliParseError::InvalidNumber(args[1].clone()))?;
        return Ok(CliCommand::SpecRepair { number });
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
        return Ok(CliCommand::SpecRename { number, title });
    }
    if let Some(section) = edit_section {
        if json {
            return Ok(CliCommand::SpecEditSectionJson {
                number,
                section,
                file,
                replace,
            });
        }
        return Ok(CliCommand::SpecEditSection {
            number,
            section,
            file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        });
    }
    if json || replace {
        return Err(CliParseError::Usage);
    }
    if let Some(section) = section {
        return Ok(CliCommand::SpecReadSection { number, section });
    }
    Ok(CliCommand::SpecReadAll { number })
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: CliCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match cmd {
        CliCommand::SpecReadAll { number } => {
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
        CliCommand::SpecReadSection { number, section } => {
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
        CliCommand::SpecEditSection {
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
        CliCommand::SpecEditSectionJson {
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
        CliCommand::SpecList { phase, state } => {
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
        CliCommand::SpecCreate {
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
        CliCommand::SpecCreateJson {
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
        CliCommand::SpecCreateHelp => {
            out.push_str(SPEC_CREATE_HELP);
            if !SPEC_CREATE_HELP.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        CliCommand::SpecPull { all, numbers } => {
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
        CliCommand::SpecRepair { number } => {
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
        CliCommand::SpecRename { number, title } => {
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

fn read_cli_input<E: CliEnv>(env: &mut E, file: Option<&str>) -> Result<String, SpecOpsError> {
    match file {
        None | Some("-") => env.read_stdin().map_err(super::io_as_api_error),
        Some(path) => env.read_file(path).map_err(super::io_as_api_error),
    }
}

fn parse_structured_spec_json(raw: &str) -> Result<StructuredSpecInput, SpecOpsError> {
    serde_json::from_str(raw).map_err(|err| {
        SpecOpsError::from(ApiError::Unexpected(format!("invalid spec json: {err}")))
    })
}

fn render_structured_spec(title: &str, structured: &StructuredSpecInput) -> String {
    let mut parts = vec![format!("# {}", title.trim())];
    if let Some(section) = structured
        .background
        .as_ref()
        .and_then(render_background_section)
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .user_stories
        .as_ref()
        .and_then(|stories| render_user_stories_section(stories))
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .edge_cases
        .as_ref()
        .and_then(|items| render_bullet_section("Edge Cases", items))
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .functional_requirements
        .as_ref()
        .and_then(|items| {
            render_numbered_requirement_section("Functional Requirements", "FR", items)
        })
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .non_functional_requirements
        .as_ref()
        .and_then(|items| {
            render_numbered_requirement_section("Non-Functional Requirements", "NFR", items)
        })
    {
        parts.push(section);
    }
    if let Some(section) = structured
        .success_criteria
        .as_ref()
        .and_then(|items| render_numbered_requirement_section("Success Criteria", "SC", items))
    {
        parts.push(section);
    }
    parts.join("\n\n").trim_end().to_string() + "\n"
}

fn merge_structured_spec(existing: &str, patch: &StructuredSpecInput) -> String {
    let (title, known_sections, unknown_sections) = split_structured_spec(existing);
    let mut merged = known_sections;
    for (heading, value) in rendered_sections_for_patch(patch) {
        match value {
            Some(content) => {
                merged.insert(heading.to_string(), content);
            }
            None => {
                merged.remove(heading);
            }
        }
    }

    let mut parts = vec![format!("# {}", title.trim())];
    for heading in CANONICAL_SECTION_HEADINGS {
        if let Some(section) = merged.get(heading) {
            parts.push(section.clone());
        }
    }
    for section in unknown_sections {
        parts.push(section);
    }
    parts.join("\n\n").trim_end().to_string() + "\n"
}

fn rendered_sections_for_patch(patch: &StructuredSpecInput) -> Vec<(&'static str, Option<String>)> {
    let mut sections = Vec::new();
    if let Some(background) = patch.background.as_ref() {
        sections.push(("Background", render_background_section(background)));
    }
    if let Some(user_stories) = patch.user_stories.as_ref() {
        sections.push(("User Stories", render_user_stories_section(user_stories)));
    }
    if let Some(edge_cases) = patch.edge_cases.as_ref() {
        sections.push((
            "Edge Cases",
            render_bullet_section("Edge Cases", edge_cases),
        ));
    }
    if let Some(requirements) = patch.functional_requirements.as_ref() {
        sections.push((
            "Functional Requirements",
            render_numbered_requirement_section("Functional Requirements", "FR", requirements),
        ));
    }
    if let Some(requirements) = patch.non_functional_requirements.as_ref() {
        sections.push((
            "Non-Functional Requirements",
            render_numbered_requirement_section("Non-Functional Requirements", "NFR", requirements),
        ));
    }
    if let Some(criteria) = patch.success_criteria.as_ref() {
        sections.push((
            "Success Criteria",
            render_numbered_requirement_section("Success Criteria", "SC", criteria),
        ));
    }
    sections
}

fn split_structured_spec(existing: &str) -> (String, BTreeMap<String, String>, Vec<String>) {
    let title = extract_document_title(existing).unwrap_or_else(|| "Specification".to_string());
    let mut known_sections = BTreeMap::new();
    let mut unknown_sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();

    let mut flush = |heading: Option<String>, lines: &mut Vec<String>| {
        if let Some(name) = heading {
            let section = lines.join("\n").trim().to_string();
            if !section.is_empty() {
                if CANONICAL_SECTION_HEADINGS.contains(&name.as_str()) {
                    known_sections.insert(name, section);
                } else {
                    unknown_sections.push(section);
                }
            }
            lines.clear();
        }
    };

    for line in existing.lines() {
        if line.starts_with("# ") && current_heading.is_none() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            let previous_heading = current_heading.take();
            flush(previous_heading, &mut current_lines);
            current_heading = Some(rest.trim().to_string());
            current_lines.push(line.to_string());
        } else if current_heading.is_some() {
            current_lines.push(line.to_string());
        }
    }
    flush(current_heading.take(), &mut current_lines);

    (title, known_sections, unknown_sections)
}

fn extract_document_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_spec_heading_from_title(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(rest) = trimmed.strip_prefix("SPEC:") {
        return rest.trim().to_string();
    }
    if let Some(colon) = trimmed.find(':') {
        let head = trimmed[..colon].trim();
        if head.eq_ignore_ascii_case("SPEC") || head.starts_with("SPEC-") {
            return trimmed[colon + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn render_background_section(background: &TextBlock) -> Option<String> {
    let content = normalize_text_block(background)?;
    Some(format!("## Background\n\n{content}"))
}

fn render_user_stories_section(user_stories: &[StructuredUserStory]) -> Option<String> {
    let rendered: Vec<String> = user_stories
        .iter()
        .enumerate()
        .filter_map(|(index, story)| render_user_story(index + 1, story))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## User Stories\n\n{}", rendered.join("\n\n")))
    }
}

fn render_user_story(index: usize, story: &StructuredUserStory) -> Option<String> {
    let title = normalize_user_story_title(&story.title);
    if title.is_empty() {
        return None;
    }
    let mut header = format!("### US-{index}: {title}");
    if let Some(priority) = story
        .priority
        .as_deref()
        .map(normalize_priority)
        .filter(|value| !value.is_empty())
    {
        header.push_str(&format!(" ({priority})"));
    }
    if let Some(status) = story
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        header.push_str(&format!(" -- {status}"));
    }

    let statement = story
        .statement
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| build_user_story_statement(story))
        .unwrap_or_else(|| "[NEEDS CLARIFICATION: add a user story statement]".to_string());

    let scenarios: Vec<String> = story
        .acceptance_scenarios
        .iter()
        .map(|scenario| strip_list_marker(scenario))
        .filter(|scenario| !scenario.is_empty())
        .collect();

    let mut parts = vec![header, String::new(), statement];
    if !scenarios.is_empty() {
        parts.push(String::new());
        parts.push("**Acceptance Scenarios:**".to_string());
        parts.push(String::new());
        parts.extend(
            scenarios
                .into_iter()
                .enumerate()
                .map(|(idx, scenario)| format!("{}. {scenario}", idx + 1)),
        );
    }
    Some(parts.join("\n"))
}

fn build_user_story_statement(story: &StructuredUserStory) -> Option<String> {
    let as_a = story.as_a.as_deref()?.trim();
    let i_want = story.i_want.as_deref()?.trim();
    let so_that = story.so_that.as_deref()?.trim();
    if as_a.is_empty() || i_want.is_empty() || so_that.is_empty() {
        return None;
    }
    Some(format!("As {as_a}, I want {i_want}, so that {so_that}."))
}

fn render_bullet_section(heading: &str, items: &[String]) -> Option<String> {
    let rendered: Vec<String> = items
        .iter()
        .map(|item| strip_list_marker(item))
        .filter(|item| !item.is_empty())
        .map(|item| format!("- {item}"))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## {heading}\n\n{}", rendered.join("\n")))
    }
}

fn render_numbered_requirement_section(
    heading: &str,
    prefix: &str,
    items: &[String],
) -> Option<String> {
    let rendered: Vec<String> = items
        .iter()
        .map(|item| strip_requirement_label(item))
        .filter(|item| !item.is_empty())
        .enumerate()
        .map(|(index, item)| format!("- **{prefix}-{number:03}**: {item}", number = index + 1))
        .collect();
    if rendered.is_empty() {
        None
    } else {
        Some(format!("## {heading}\n\n{}", rendered.join("\n")))
    }
}

fn normalize_text_block(text: &TextBlock) -> Option<String> {
    match text {
        TextBlock::Text(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        TextBlock::Paragraphs(values) => {
            let paragraphs: Vec<String> = values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if paragraphs.is_empty() {
                None
            } else {
                Some(paragraphs.join("\n\n"))
            }
        }
    }
}

fn normalize_user_story_title(title: &str) -> String {
    let trimmed = title.trim();
    if let Some(colon) = trimmed.find(':') {
        let head = trimmed[..colon].trim();
        if head.starts_with("US-") {
            return trimmed[colon + 1..].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn normalize_priority(priority: &str) -> String {
    let trimmed = priority.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with('P') {
        upper
    } else {
        format!("P{upper}")
    }
}

fn strip_list_marker(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return rest.trim().to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return rest.trim().to_string();
    }
    if let Some((head, rest)) = trimmed.split_once(". ") {
        if !head.is_empty() && head.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn strip_requirement_label(value: &str) -> String {
    let without_list = strip_list_marker(value);
    let mut candidate = without_list.trim().trim_matches('*').trim().to_string();
    if let Some((head, rest)) = candidate.split_once(':') {
        let label = head.trim().trim_matches('*');
        if label.starts_with("FR-") || label.starts_with("NFR-") || label.starts_with("SC-") {
            return rest.trim().to_string();
        }
    }
    if candidate.starts_with("**") && candidate.contains("**:") {
        candidate = candidate.replacen("**", "", 1);
        candidate = candidate.replacen("**", "", 1);
        if let Some((head, rest)) = candidate.split_once(':') {
            let label = head.trim();
            if label.starts_with("FR-") || label.starts_with("NFR-") || label.starts_with("SC-") {
                return rest.trim().to_string();
            }
        }
    }
    without_list
}
