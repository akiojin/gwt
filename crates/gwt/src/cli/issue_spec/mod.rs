//! Issue SPEC command model used by JSON envelope operations (SPEC-1942 SC-027 split).
//!
//! Hosts argv `parse` and dispatch `run` for the SPEC subcommand surface
//! plus the family `tests` block. Structured JSON model and render / merge
//! helpers live in [`structured`].

mod structured;

use gwt_github::{
    client::ApiError, Cache, IssueClient, IssueNumber, SectionName, SpecListFilter, SpecOps,
    SpecOpsError,
};

use crate::cli::{
    issue::guard_autonomous_acceptance_block, CliEnv, CliParseError, ClientRef, IssueCommand,
};

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
const SPEC_CREATE_HELP: &str = r##"gwtd stdin JSON envelope operation: issue.spec.create

Params: title (required), body, labels, structured, allow_empty.

- body (Markdown, the default): the SPEC text. A body with no
  `<!-- artifact:NAME BEGIN -->` markers is stored whole as the `spec`
  section; a body that carries markers keeps the sections it declares.
- structured: true — `body` is the structured JSON object described below
  instead of Markdown.
- allow_empty: true — create a SPEC with no section content on purpose.
  Without it, a create that would write nothing is refused (Issue #4250).

There is no `params.spec`; structured input travels in `body` with
`structured: true`.

Structured SPEC input is owned by gwt-github (spec_structured). Use this help
output as the single source of truth for that JSON shape and the generated
Markdown format.

Input JSON schema (the value of `body` when `structured` is true):
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
"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: Launch agents from GUI","body":"# SPEC: Launch agents from GUI\n\n## 背景\n..."}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.create","params":{"title":"SPEC: Launch agents from GUI","structured":true,"body":{"background":["..."],"user_stories":[],"functional_requirements":[],"success_criteria":[]}}}
JSON

"$GWT_BIN" <<'JSON'
{"schema_version":1,"operation":"issue.spec.edit","params":{"number":1942,"section":"spec","structured":true,"body":{"background":["..."],"user_stories":[],"functional_requirements":[],"success_criteria":[]},"replace":true}}
JSON
"##;

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
    if head == "audit" {
        let mut state: Option<String> = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
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
        return Ok(IssueCommand::SpecAudit { state });
    }
    if head == "create" {
        let mut title: Option<String> = None;
        let mut file: Option<String> = None;
        let mut labels: Vec<String> = Vec::new();
        let mut json = false;
        let mut help = false;
        let mut allow_empty = false;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--help" => help = true,
                "--json" => json = true,
                "--allow-empty" => allow_empty = true,
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
            allow_empty,
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
            write_spec_section(env, out, number, section, content)?
        }
        IssueCommand::SpecEditSectionBody {
            number,
            section,
            body,
        } => write_spec_section(env, out, number, section, body)?,
        IssueCommand::SpecEditSectionJsonBody {
            number,
            section,
            body,
            replace,
        } => {
            if section != SPEC_SECTION_NAME {
                return Err(SpecOpsError::from(ApiError::Unexpected(format!(
                    "structured JSON edit only supports section '{SPEC_SECTION_NAME}'"
                ))));
            }
            write_structured_spec_section(env, out, number, section, &body, replace)?
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
            write_structured_spec_section(env, out, number, section, &raw, replace)?
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
        IssueCommand::SpecAudit { state } => audit_spec_tasks(env, out, state.as_deref())?,
        IssueCommand::SpecCreate {
            title,
            file,
            labels,
            allow_empty,
        } => {
            let raw = env.read_file(&file).map_err(|err| {
                SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
            })?;
            create_spec_from_markdown(env, out, title, &raw, labels, allow_empty)?
        }
        IssueCommand::SpecCreateBody {
            title,
            body,
            labels,
            allow_empty,
        } => create_spec_from_markdown(env, out, title, &body, labels, allow_empty)?,
        IssueCommand::SpecCreateJsonBody {
            title,
            body,
            labels,
        } => create_spec_from_structured_json(env, out, title, &body, labels)?,
        IssueCommand::SpecCreateJson {
            title,
            file,
            labels,
        } => {
            let raw = read_cli_input(env, file.as_deref())?;
            create_spec_from_structured_json(env, out, title, &raw, labels)?
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

/// Issue #4146 AC-4: report every gwt-spec Issue whose `tasks` section carries
/// task rows with no checkbox.
///
/// Those rows used to be invisible to the completion count, so a section could
/// read "10 done / 0 open" with 51 rows untouched and the Issue was closed as
/// complete. Closed Issues are scanned by default because that is where the
/// damage already landed; `--state open` / `--state all` widen the scan.
fn audit_spec_tasks<E: CliEnv>(
    env: &mut E,
    out: &mut String,
    state: Option<&str>,
) -> Result<i32, SpecOpsError> {
    let state = match state.unwrap_or("closed") {
        "closed" => Some(gwt_github::client::IssueState::Closed),
        "open" => Some(gwt_github::client::IssueState::Open),
        "all" => None,
        other => {
            return Err(SpecOpsError::from(ApiError::Unexpected(format!(
                "unknown --state '{other}' (expected open, closed, or all)"
            ))))
        }
    };
    let cache = Cache::new(env.cache_root());
    let ops = SpecOps::new(
        ClientRef {
            inner: env.client(),
        },
        cache,
    );
    let list = env
        .client()
        .list_spec_issues(&SpecListFilter { phase: None, state })?;
    let scanned = list.len();
    let tasks = SectionName("tasks".to_string());
    let mut flagged = 0usize;
    let mut without_tasks = 0usize;
    let mut unreadable: Vec<String> = Vec::new();
    for spec in list {
        let content = match ops.read_section(spec.number, &tasks) {
            Ok(content) => content,
            Err(SpecOpsError::SectionNotFound(_)) => {
                without_tasks += 1;
                continue;
            }
            Err(err) => {
                unreadable.push(format!("#{}: {err}", spec.number.0));
                continue;
            }
        };
        let progress = crate::spec_tasks::parse_tasks_progress(&content);
        if progress.untracked == 0 {
            continue;
        }
        flagged += 1;
        let state_marker = match spec.state {
            gwt_github::client::IssueState::Open => "OPEN",
            gwt_github::client::IssueState::Closed => "CLOSED",
        };
        out.push_str(&format!(
            "#{} [{state_marker}] completed={} open={} untracked={} {}\n",
            spec.number.0, progress.completed, progress.open, progress.untracked, spec.title
        ));
    }
    for failure in &unreadable {
        out.push_str(&format!("unreadable {failure}\n"));
    }
    out.push_str(&format!(
        "audit: scanned {scanned}, flagged {flagged}, no tasks section {without_tasks}, unreadable {}\n",
        unreadable.len()
    ));
    Ok(0)
}

fn write_spec_section<E: CliEnv>(
    env: &mut E,
    out: &mut String,
    number: u64,
    section: String,
    content: String,
) -> Result<i32, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    let ops = SpecOps::new(
        ClientRef {
            inner: env.client(),
        },
        cache,
    );
    guard_spec_section_write(&ops, number, &section, &content)?;
    let receipt =
        ops.write_section(IssueNumber(number), &SectionName(section.clone()), &content)?;
    super::intake_outcome::auto_record_issue_operation(
        env.repo_path(),
        "issue.spec.edit",
        super::intake_outcome::IntakeOutcomeKind::SpecUpdated,
        number,
    );
    // T-274: verified writes feed the per-owner operability ledger.
    super::artifact_operability::record_write_best_effort(
        env.repo_path(),
        number,
        &section,
        &receipt,
    );
    out.push_str(&render_write_receipt(&section, &receipt));
    Ok(0)
}

/// Issue #3873 AC-2: a `spec` section write on an `auto-merge` Issue must keep
/// a machine-checkable acceptance block. Reads the Issue's current labels
/// through a conditional refresh so a label added after the last cache write
/// is honoured; other sections are never inspected.
fn guard_spec_section_write<C: IssueClient>(
    ops: &SpecOps<C>,
    number: u64,
    section: &str,
    content: &str,
) -> Result<(), SpecOpsError> {
    if section != SPEC_SECTION_NAME {
        return Ok(());
    }
    ops.refresh_cache(IssueNumber(number))?;
    let labels = ops
        .cache()
        .load_entry(IssueNumber(number))
        .map(|entry| entry.snapshot.labels)
        .unwrap_or_default();
    guard_autonomous_acceptance_block(&labels, content)
}

/// Render the committed-write evidence line (SPEC-3248 P7C / #3284): byte
/// count, storage shape, content hash, and the readback confirmation.
fn render_write_receipt(section: &str, receipt: &gwt_github::WriteReceipt) -> String {
    let shape = match receipt.parts {
        0 => "body".to_string(),
        1 => "1 comment part".to_string(),
        n => format!("{n} comment parts"),
    };
    format!(
        "wrote {} bytes to section '{section}' ({shape}, sha256:{}, readback verified)\n",
        receipt.bytes, receipt.sha256
    )
}

fn write_structured_spec_section<E: CliEnv>(
    env: &mut E,
    out: &mut String,
    number: u64,
    section: String,
    raw_json: &str,
    replace: bool,
) -> Result<i32, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    let ops = SpecOps::new(
        ClientRef {
            inner: env.client(),
        },
        cache,
    );
    let structured = parse_structured_spec_json(raw_json)?;
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
    guard_spec_section_write(&ops, number, &section, &content)?;
    let receipt =
        ops.write_section(IssueNumber(number), &SectionName(section.clone()), &content)?;
    super::intake_outcome::auto_record_issue_operation(
        env.repo_path(),
        "issue.spec.edit",
        super::intake_outcome::IntakeOutcomeKind::SpecUpdated,
        number,
    );
    super::artifact_operability::record_write_best_effort(
        env.repo_path(),
        number,
        &section,
        &receipt,
    );
    out.push_str(&render_write_receipt(&section, &receipt));
    Ok(0)
}

/// Decide which sections a create writes, and refuse the shapes that used to
/// produce an empty SPEC under `ok:true` (Issue #4250 AC-1 / AC-3).
///
/// The silent drop came from treating [`gwt_github::extract_sections`] as the
/// only source of sections: plain Markdown carries no
/// `<!-- artifact:NAME BEGIN -->` markers, so the parser returned nothing and
/// the body went nowhere. An unmarked body is not "no sections" — it is the
/// SPEC text, so it becomes the `spec` section. Anything that really would
/// write nothing is refused unless the caller asked for an empty SPEC.
fn resolve_create_sections(
    raw: &str,
    allow_empty: bool,
) -> Result<BTreeMap<SectionName, String>, SpecOpsError> {
    let parsed = gwt_github::extract_sections(raw)
        .map_err(|err| SpecOpsError::from(gwt_github::body::ParseError::Section(err)))?;
    if parsed.is_empty() {
        if raw.trim().is_empty() {
            return if allow_empty {
                Ok(BTreeMap::new())
            } else {
                Err(empty_spec_create_refusal("the body is empty"))
            };
        }
        return Ok(BTreeMap::from([(
            SectionName(SPEC_SECTION_NAME.to_string()),
            raw.to_string(),
        )]));
    }
    let sections: BTreeMap<SectionName, String> = parsed
        .into_iter()
        .map(|section| (section.name, section.content))
        .collect();
    if !allow_empty && sections.values().all(|content| content.trim().is_empty()) {
        return Err(empty_spec_create_refusal(
            "every section marker in the body encloses empty content",
        ));
    }
    Ok(sections)
}

/// Pre-write refusal: nothing was created, so the caller fixes the input and
/// retries. Names the opt-in because creating an empty SPEC on purpose is a
/// real (if rare) use, and a refusal with no escape hatch just moves the
/// confusion one step.
fn empty_spec_create_refusal(detail: &str) -> SpecOpsError {
    SpecOpsError::Validation(format!(
        "issue.spec.create refused: {detail}, so no section would be written and the \
         SPEC would be created empty. Pass the SPEC text in `body`, or pass \
         `allow_empty: true` to create an empty SPEC on purpose."
    ))
}

fn create_spec_from_markdown<E: CliEnv>(
    env: &mut E,
    out: &mut String,
    title: String,
    raw: &str,
    labels: Vec<String>,
    allow_empty: bool,
) -> Result<i32, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    let ops = SpecOps::new(
        ClientRef {
            inner: env.client(),
        },
        cache,
    );
    let sections = resolve_create_sections(raw, allow_empty)?;
    guard_autonomous_acceptance_block(&labels, raw)?;
    let written: Vec<String> = sections.keys().map(|name| name.0.clone()).collect();
    let snapshot = ops.create_spec(&title, sections, &labels)?;
    super::intake_outcome::auto_record_issue_operation(
        env.repo_path(),
        "issue.spec.create",
        super::intake_outcome::IntakeOutcomeKind::SpecCreated,
        snapshot.number.0,
    );
    out.push_str(&format!(
        "created issue #{} with labels {:?}{}\n",
        snapshot.number.0,
        snapshot.labels,
        render_created_sections(&written)
    ));
    Ok(0)
}

/// Issue #4250 AC-4: say which sections the create actually stored. The
/// operation used to report only the Issue number, which read identically
/// whether the body landed or vanished.
fn render_created_sections(written: &[String]) -> String {
    if written.is_empty() {
        " (no sections written, allow_empty)".to_string()
    } else {
        format!(" (sections [{}] readback verified)", written.join(", "))
    }
}

fn create_spec_from_structured_json<E: CliEnv>(
    env: &mut E,
    out: &mut String,
    title: String,
    raw_json: &str,
    labels: Vec<String>,
) -> Result<i32, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    let ops = SpecOps::new(
        ClientRef {
            inner: env.client(),
        },
        cache,
    );
    let structured = parse_structured_spec_json(raw_json)?;
    let spec = render_structured_spec(&normalize_spec_heading_from_title(&title), &structured);
    guard_autonomous_acceptance_block(&labels, &spec)?;
    let sections = BTreeMap::from([(SectionName(SPEC_SECTION_NAME.to_string()), spec)]);
    let snapshot = ops.create_spec(&title, sections, &labels)?;
    super::intake_outcome::auto_record_issue_operation(
        env.repo_path(),
        "issue.spec.create",
        super::intake_outcome::IntakeOutcomeKind::SpecCreated,
        snapshot.number.0,
    );
    out.push_str(&format!(
        "created issue #{} with labels {:?}\n",
        snapshot.number.0, snapshot.labels
    ));
    Ok(0)
}

#[cfg(test)]
mod tests;
