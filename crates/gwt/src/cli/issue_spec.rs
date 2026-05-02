use std::collections::BTreeMap;

use gwt_github::{
    client::ApiError, Cache, IssueClient, IssueNumber, SectionName, SpecListFilter, SpecOps,
    SpecOpsError,
};
use serde::Deserialize;

use crate::cli::{CliEnv, CliParseError, ClientRef, IssueCommand};

const SPEC_SECTION_NAME: &str = "spec";
const CANONICAL_SECTION_HEADINGS: [&str; 6] = [
    "Background",
    "User Stories",
    "Edge Cases",
    "Functional Requirements",
    "Non-Functional Requirements",
    "Success Criteria",
];
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

#[cfg(test)]
mod tests {
    use gwt_github::client::{IssueSnapshot, IssueState, UpdatedAt};

    use crate::cli::env::TestEnv;

    use super::*;

    fn sample_structured_input() -> StructuredSpecInput {
        StructuredSpecInput {
            background: Some(TextBlock::Paragraphs(vec![
                "First paragraph.".to_string(),
                "Second paragraph.".to_string(),
            ])),
            user_stories: Some(vec![StructuredUserStory {
                title: "US-9: Launch agent".to_string(),
                priority: Some("1".to_string()),
                status: Some("READY".to_string()),
                statement: None,
                as_a: Some("developer".to_string()),
                i_want: Some("a launch workflow".to_string()),
                so_that: Some("I can start quickly".to_string()),
                acceptance_scenarios: vec![
                    "- Given a branch, when I open the wizard, then it lists agent options"
                        .to_string(),
                    "2. Given Docker support, when selected, then runtime options appear"
                        .to_string(),
                ],
            }]),
            edge_cases: Some(vec!["- Missing branch".to_string()]),
            functional_requirements: Some(vec!["FR-999: Launch selected agent".to_string()]),
            non_functional_requirements: Some(vec!["Low latency".to_string()]),
            success_criteria: Some(vec!["1. Users can launch an agent".to_string()]),
        }
    }

    fn issue_body(spec: &str, tasks: &str) -> String {
        format!(
            "<!-- gwt-spec id=42 version=1 -->\n\
<!-- sections:\n\
spec=body\n\
tasks=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
{spec}\n\
<!-- artifact:spec END -->\n\
\n\
<!-- artifact:tasks BEGIN -->\n\
{tasks}\n\
<!-- artifact:tasks END -->\n"
        )
    }

    fn seed_issue(
        env: &TestEnv,
        number: u64,
        title: &str,
        spec: &str,
        tasks: &str,
        labels: &[&str],
    ) {
        let snapshot = IssueSnapshot {
            number: IssueNumber(number),
            title: title.to_string(),
            body: issue_body(spec, tasks),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new(format!("seed-{number}")),
            comments: Vec::new(),
        };
        env.client.seed(snapshot.clone());
        Cache::new(env.cache_root())
            .write_snapshot(&snapshot)
            .unwrap();
    }

    #[test]
    fn parse_and_render_structured_spec_include_all_sections() {
        let parsed = parse_structured_spec_json(
            r#"{
                "background": ["First paragraph.", "Second paragraph."],
                "user_stories": [{
                    "title": "Launch agent",
                    "priority": "P0",
                    "statement": "As a developer, I want to launch an agent, so that I can work faster.",
                    "acceptance_scenarios": ["Given a branch, when I launch, then the agent starts"]
                }],
                "edge_cases": ["Missing branch"],
                "functional_requirements": ["Launch selected agent"],
                "non_functional_requirements": ["Low latency"],
                "success_criteria": ["Agents launch from the selected branch"]
            }"#,
        )
        .expect("parse structured json");
        let rendered = render_structured_spec("Launch agents from GUI", &parsed);

        assert!(rendered.starts_with("# Launch agents from GUI\n"));
        assert!(rendered.contains("## Background"));
        assert!(rendered.contains("First paragraph.\n\nSecond paragraph."));
        assert!(rendered.contains("## User Stories"));
        assert!(rendered.contains("### US-1: Launch agent (P0)"));
        assert!(rendered.contains("**Acceptance Scenarios:**"));
        assert!(rendered.contains("## Edge Cases"));
        assert!(rendered.contains("- Missing branch"));
        assert!(rendered.contains("## Functional Requirements"));
        assert!(rendered.contains("- **FR-001**: Launch selected agent"));
        assert!(rendered.contains("## Non-Functional Requirements"));
        assert!(rendered.contains("- **NFR-001**: Low latency"));
        assert!(rendered.contains("## Success Criteria"));
        assert!(rendered.contains("- **SC-001**: Agents launch from the selected branch"));

        let err = parse_structured_spec_json("{not-json").unwrap_err();
        assert!(err.to_string().contains("invalid spec json"));
    }

    #[test]
    fn merge_structured_spec_updates_known_sections_and_preserves_unknown_content() {
        let existing = r#"# SPEC: Launch agents

## Background

Old background.

## User Stories

### US-1: Old story

Old statement.

## Custom Notes

Keep this note.
"#;
        let patch = StructuredSpecInput {
            background: Some(TextBlock::Text("".to_string())),
            user_stories: sample_structured_input().user_stories,
            edge_cases: Some(vec!["New edge".to_string()]),
            functional_requirements: None,
            non_functional_requirements: None,
            success_criteria: None,
        };

        let merged = merge_structured_spec(existing, &patch);

        assert!(merged.starts_with("# SPEC: Launch agents\n"));
        assert!(!merged.contains("## Background"));
        assert!(merged.contains("### US-1: Launch agent (P1) -- READY"));
        assert!(merged.contains("## Edge Cases"));
        assert!(merged.contains("- New edge"));
        assert!(merged.contains("## Custom Notes"));
        assert!(merged.contains("Keep this note."));
    }

    #[test]
    fn split_and_normalize_helpers_strip_labels_and_build_story_text() {
        let existing = r#"# SPEC-77: Launch agents

## Background

Background text.

## User Stories

### US-1: Existing story

Statement.

## Extra

Preserve me.
"#;

        let (title, known, unknown) = split_structured_spec(existing);
        assert_eq!(title, "SPEC-77: Launch agents");
        assert_eq!(
            extract_document_title(existing),
            Some("SPEC-77: Launch agents".to_string())
        );
        assert_eq!(
            normalize_spec_heading_from_title("SPEC-77: Launch agents"),
            "Launch agents"
        );
        assert!(known.contains_key("Background"));
        assert!(known.contains_key("User Stories"));
        assert_eq!(unknown, vec!["## Extra\n\nPreserve me.".to_string()]);

        assert_eq!(
            build_user_story_statement(
                &sample_structured_input()
                    .user_stories
                    .unwrap()
                    .into_iter()
                    .next()
                    .unwrap()
            ),
            Some(
                "As developer, I want a launch workflow, so that I can start quickly.".to_string()
            )
        );
        assert_eq!(
            normalize_user_story_title("US-9: Launch agent"),
            "Launch agent"
        );
        assert_eq!(normalize_priority("1"), "P1");
        assert_eq!(strip_list_marker("2. Listed item"), "Listed item");
        assert_eq!(strip_list_marker("- Bullet item"), "Bullet item");
        assert_eq!(
            strip_requirement_label("- **FR-001**: Requirement text"),
            "Requirement text"
        );
        assert_eq!(
            render_background_section(&TextBlock::Text("  ".to_string())),
            None
        );
        assert_eq!(
            render_bullet_section("Edge Cases", &["".to_string(), "- case".to_string()]),
            Some("## Edge Cases\n\n- case".to_string())
        );
        assert_eq!(
            render_numbered_requirement_section(
                "Functional Requirements",
                "FR",
                &["FR-009: Requirement".to_string()],
            ),
            Some("## Functional Requirements\n\n- **FR-001**: Requirement".to_string())
        );
    }

    #[test]
    fn read_cli_input_uses_stdin_and_named_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = TestEnv::new(temp.path().to_path_buf());
        env.stdin = "from-stdin".to_string();
        env.files
            .insert("spec.json".to_string(), "from-file".to_string());

        assert_eq!(read_cli_input(&mut env, None).unwrap(), "from-stdin");
        assert_eq!(
            read_cli_input(&mut env, Some("spec.json")).unwrap(),
            "from-file"
        );
    }

    #[test]
    fn parse_supports_list_create_pull_repair_and_edit_modes() {
        let args = [
            "list".to_string(),
            "--phase".to_string(),
            "review".to_string(),
            "--state".to_string(),
            "closed".to_string(),
        ];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecList { phase, state })
                if phase.as_deref() == Some("review") && state.as_deref() == Some("closed")
        ));

        let args = [
            "create".to_string(),
            "--json".to_string(),
            "--title".to_string(),
            "SPEC: Launch".to_string(),
            "--label".to_string(),
            "phase/review".to_string(),
            "-f".to_string(),
            "spec.json".to_string(),
        ];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecCreateJson { title, file, labels })
                if title == "SPEC: Launch"
                    && file.as_deref() == Some("spec.json")
                    && labels == vec!["phase/review".to_string()]
        ));

        let args = ["create".to_string(), "--help".to_string()];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(parse(&refs), Ok(IssueCommand::SpecCreateHelp)));

        let args = [
            "1942".to_string(),
            "--edit".to_string(),
            "spec".to_string(),
            "--json".to_string(),
            "--replace".to_string(),
            "-f".to_string(),
            "update.json".to_string(),
        ];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecEditSectionJson {
                number,
                section,
                file,
                replace
            })
                if number == 1942
                    && section == "spec"
                    && file.as_deref() == Some("update.json")
                    && replace
        ));

        let args = ["pull".to_string(), "--all".to_string(), "77".to_string()];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecPull { all, numbers }) if all && numbers == vec![77]
        ));

        let args = ["repair".to_string(), "77".to_string()];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecRepair { number }) if number == 77
        ));

        let args = [
            "77".to_string(),
            "--rename".to_string(),
            "SPEC: Renamed".to_string(),
        ];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Ok(IssueCommand::SpecRename { number, title })
                if number == 77 && title == "SPEC: Renamed"
        ));

        let args = ["create".to_string(), "--title".to_string()];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Err(CliParseError::MissingFlag("--title"))
        ));

        let args = [
            "77".to_string(),
            "--rename".to_string(),
            "SPEC: Renamed".to_string(),
            "--json".to_string(),
        ];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(parse(&refs), Err(CliParseError::Usage)));

        let args = ["list".to_string(), "--bogus".to_string()];
        let refs = args.iter().collect::<Vec<_>>();
        assert!(matches!(
            parse(&refs),
            Err(CliParseError::UnknownSubcommand(value)) if value == "--bogus"
        ));
    }

    #[test]
    fn run_supports_read_create_pull_repair_and_rename_workflows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = TestEnv::new(temp.path().to_path_buf());
        seed_issue(
            &env,
            42,
            "SPEC: Launch agents",
            "spec body",
            "tasks body",
            &["gwt-spec", "phase/review"],
        );

        let mut out = String::new();
        assert_eq!(
            run(&mut env, IssueCommand::SpecReadAll { number: 42 }, &mut out).unwrap(),
            0
        );
        assert!(out.contains("=== spec ===\nspec body"));
        assert!(out.contains("=== tasks ===\ntasks body"));

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecReadSection {
                    number: 42,
                    section: "tasks".to_string(),
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert_eq!(out, "tasks body\n");

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecList {
                    phase: Some("review".to_string()),
                    state: Some("open".to_string()),
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert!(out.contains("#42 [OPEN] [phase/review] SPEC: Launch agents"));

        env.files.insert(
            "legacy.md".to_string(),
            issue_body("created spec", "created tasks"),
        );
        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecCreate {
                    title: "SPEC: Created from markdown".to_string(),
                    file: "legacy.md".to_string(),
                    labels: vec!["gwt-spec".to_string()],
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert!(out.contains("created issue #43"));

        env.stdin = serde_json::json!({
            "background": "Created from json",
            "success_criteria": ["Agents launch from CLI"]
        })
        .to_string();
        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecCreateJson {
                    title: "SPEC: Created from json".to_string(),
                    file: None,
                    labels: vec!["gwt-spec".to_string()],
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert!(out.contains("created issue #44"));

        out.clear();
        assert_eq!(
            run(&mut env, IssueCommand::SpecCreateHelp, &mut out).unwrap(),
            0
        );
        assert!(out.contains("Input JSON schema:"));

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecPull {
                    all: true,
                    numbers: Vec::new(),
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert_eq!(out, "pulled all gwt-spec issues\n");

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecPull {
                    all: false,
                    numbers: vec![42],
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert_eq!(out, "pulled #42\n");

        let err = run(
            &mut env,
            IssueCommand::SpecPull {
                all: false,
                numbers: Vec::new(),
            },
            &mut out,
        )
        .unwrap_err();
        assert!(err.to_string().contains("pull requires --all or <n>"));

        out.clear();
        assert_eq!(
            run(&mut env, IssueCommand::SpecRepair { number: 42 }, &mut out).unwrap(),
            0
        );
        assert_eq!(out, "repaired cache for #42\n");

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecRename {
                    number: 42,
                    title: "SPEC: Renamed".to_string(),
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert!(out.contains("renamed issue #42 to 'SPEC: Renamed'"));
        let cache = Cache::new(env.cache_root());
        assert_eq!(
            cache.load_entry(IssueNumber(42)).unwrap().snapshot.title,
            "SPEC: Renamed"
        );
    }

    #[test]
    fn run_edit_commands_cover_plain_and_structured_json_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut env = TestEnv::new(temp.path().to_path_buf());
        seed_issue(
            &env,
            7,
            "SPEC: Launch agents",
            "# Launch agents\n\n## Background\n\nOld background.",
            "old tasks",
            &["gwt-spec", "phase/review"],
        );

        env.files
            .insert("tasks.md".to_string(), "updated tasks".to_string());
        let mut out = String::new();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecEditSection {
                    number: 7,
                    section: "tasks".to_string(),
                    file: "tasks.md".to_string(),
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        assert!(out.contains("wrote 13 bytes to section 'tasks'"));

        out.clear();
        env.stdin = serde_json::json!({
            "background": ["New background"],
            "edge_cases": ["Missing branch"]
        })
        .to_string();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecEditSectionJson {
                    number: 7,
                    section: "spec".to_string(),
                    file: None,
                    replace: false,
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        let cache = Cache::new(env.cache_root());
        let merged_body = cache.load_entry(IssueNumber(7)).unwrap().snapshot.body;
        assert!(merged_body.contains("## Background\n\nNew background"));
        assert!(merged_body.contains("## Edge Cases"));

        env.files.insert(
            "replace.json".to_string(),
            serde_json::json!({
                "background": "Replacement background",
                "success_criteria": ["Replacement criteria"]
            })
            .to_string(),
        );
        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::SpecEditSectionJson {
                    number: 7,
                    section: "spec".to_string(),
                    file: Some("replace.json".to_string()),
                    replace: true,
                },
                &mut out,
            )
            .unwrap(),
            0
        );
        let replaced_body = Cache::new(env.cache_root())
            .load_entry(IssueNumber(7))
            .unwrap()
            .snapshot
            .body;
        assert!(replaced_body.contains("# Launch agents"));
        assert!(replaced_body.contains("Replacement background"));
        assert!(replaced_body.contains("## Success Criteria"));
        assert!(!replaced_body.contains("## Edge Cases"));

        let err = run(
            &mut env,
            IssueCommand::SpecEditSectionJson {
                number: 7,
                section: "tasks".to_string(),
                file: None,
                replace: false,
            },
            &mut out,
        )
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("structured JSON edit only supports section 'spec'"));
    }
}
