//! Known-workaround advisory attached to every hook denial (Issue #4542).
//!
//! A denial tells an agent what it must not do. It does not tell it what the
//! last agent that hit the same wall actually did about it — even when that
//! answer is already written down, in the machine-local work-notes memory or in
//! an open Issue. Agents do not search for it on their own, so the knowledge
//! stays where it was captured and the same wall is rediscovered every time.
//!
//! This module derives a stable *block signature* from the denial's summary,
//! looks that signature up across both corpora, and appends the hits as a
//! `Known workarounds` section on the denial reason.
//!
//! Four properties are load-bearing:
//!
//! - **One funnel.** Denials are constructed in a dozen places across
//!   `block_*` / `workflow_policy`, but every one of them is serialized by
//!   `run_daemon_hook::write_hook_output`. The augmentation happens there, so a
//!   new gate inherits the advisory without touching this module.
//! - **Fail-open, always.** A hook runs in the agent's critical path. An absent
//!   corpus, an unreadable cache, an unwritable ledger — each one resolves to
//!   *the unchanged denial text*, never to an error and never to a stall. The
//!   advisory is a bonus; the gate is the contract.
//! - **Bounded.** The lookup runs on a worker thread against a wall-clock
//!   budget. When the budget expires the denial is emitted as-is and the worker
//!   is abandoned; it cannot delay the agent past the budget.
//! - **Local.** The lookup reads the work-notes memory file and the Issue cache
//!   directly instead of querying the semantic project index. That is a
//!   measured decision, not a shortcut: a hook is a fresh short-lived process,
//!   so an index query reloads the embedding runtime every time and cost 6.4s
//!   warm on the development machine — far past any budget a denial can carry.
//!   The same two corpora answer the same question from local files in
//!   milliseconds. [`Lookup`] keeps the seam, so a persistent-runtime semantic
//!   backend can replace this one without touching the rest of the module.
//!
//! The section deliberately speaks Issue / Execution vocabulary. The original
//! task (SPEC-3248 T-314) asked for lane-aware wording, but the lane machinery
//! was deleted in PR #3429 and no `LaneProfile` exists to be aware of.

use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::HookOutput;

/// Wall-clock budget for the whole advisory (lookup + occurrence accounting).
const DEFAULT_BUDGET_MS: u64 = 1_500;

/// Opt-out / tuning knob. `0` disables the advisory outright.
const BUDGET_ENV: &str = "GWT_HOOK_KNOWN_WORKAROUNDS_BUDGET_MS";

/// Signature tokens kept. Enough to stay distinctive between gates, few enough
/// that an incidental trailing clause cannot fork one gate into two signatures.
const SIGNATURE_TOKEN_LIMIT: usize = 8;

/// Advisory entries rendered. A denial is already long; a third page of
/// "related" hits is how an agent learns to skip the whole section.
const MAX_ENTRIES: usize = 3;

/// Share of the signature's content tokens a candidate must carry.
///
/// The alarm-fatigue guard: an advisory that fires on everything is one an
/// agent learns to scroll past, which costs more than showing nothing.
const RELEVANCE_FLOOR: f64 = 0.5;

/// Characters of a candidate's text kept in the rendered entry.
const SUMMARY_CHARS: usize = 200;

/// Cap on the memory file read. The corpus is a scratch log, not a database;
/// a pathological one must not turn a denial into a large read.
const MAX_MEMORY_BYTES: u64 = 4 * 1024 * 1024;

/// Cap on Issue cache directories scanned in one advisory.
const MAX_ISSUE_SCAN: usize = 4_000;

/// Tokens carrying no signal for either the signature or the match. Kept
/// deliberately small and generic: every word removed here is a word two
/// unrelated gates can no longer be told apart by.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "be", "been", "was", "were", "to", "of", "in", "on", "for",
    "by", "with", "and", "or", "it", "its", "this", "that", "these", "those", "at", "as", "from",
    "into", "not", "no", "do", "does", "did", "has", "have", "had", "you", "your", "must", "may",
    "can", "will", "before", "after", "than", "then", "they", "their", "them", "there", "here",
    "so", "if", "when", "while", "which", "who", "what", "how", "but", "any", "all", "each", "per",
];

/// Split `text` into the lowercase content tokens both the signature and the
/// match are computed from. Order is preserved, so a signature reads as a
/// phrase rather than a set.
fn content_tokens(text: &str) -> Vec<String> {
    gwt_core::error_ledger::sanitize_error_message(text)
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|token| token.chars().count() > 1)
        .filter(|token| !token.chars().all(|ch| ch.is_ascii_digit()))
        .filter(|token| !STOPWORDS.contains(&token.as_str()))
        .collect()
}

/// Derive the block signature for a denial summary.
///
/// The signature is the query *and* the ledger key, so it must be stable across
/// sessions and free of anything session-scoped: paths, issue numbers, PIDs.
///
/// Returns `None` when nothing survives normalization — there is no signature
/// to key on, so there is no advisory.
pub fn block_signature(summary: &str) -> Option<String> {
    let tokens = content_tokens(summary);
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .into_iter()
            .take(SIGNATURE_TOKEN_LIMIT)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// A corpus hit, before ranking and rendering.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub(crate) origin: EntryOrigin,
    pub(crate) title: String,
    /// The actionable line: what the previous agent did, not what it hit.
    pub(crate) detail: String,
    /// Share of the signature's tokens this candidate carries, `0.0..=1.0`.
    pub(crate) score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EntryOrigin {
    /// A work-notes memory entry: the surviving self-improvement capture path
    /// after the dedicated capture CLI was retired (SPEC #3164).
    CapturedMemory,
    /// An open Issue. Closed Issues are filtered out: a closed Issue's
    /// workaround is either landed (so the gate would not have fired) or moot.
    OpenIssue { number: u64 },
}

/// A rendered advisory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkaroundEntry {
    pub(crate) origin: EntryOrigin,
    pub(crate) title: String,
    pub(crate) summary: String,
}

/// Every filesystem location the advisory touches, resolved up front.
///
/// These are resolved on the calling thread and moved into the worker rather
/// than resolved inside it. The gwt home can be overridden per-thread (that is
/// how tests isolate it), so a worker that resolved its own paths would read
/// and write a different home than the session it is advising.
#[derive(Debug, Clone)]
pub(crate) struct AdvisoryPaths {
    /// `~/.gwt/projects/<repo-hash>/work-notes/` — holds the occurrence ledger.
    pub(crate) work_notes_dir: PathBuf,
    /// The memory file to read: the home file, or the legacy repo-local one.
    pub(crate) memory_file: PathBuf,
    /// This repository's Issue cache root, when it has one. Without it no Issue
    /// can be proven open, so Issue hits are dropped.
    pub(crate) issue_cache_root: Option<PathBuf>,
}

impl AdvisoryPaths {
    pub(crate) fn for_repo(repo_root: &Path) -> Self {
        Self {
            work_notes_dir: gwt_core::paths::gwt_work_notes_dir(repo_root),
            memory_file: gwt_core::paths::resolve_work_notes_memory_read_path(repo_root),
            issue_cache_root: crate::issue_cache::issue_cache_root_for_repo_path(repo_root),
        }
    }
}

/// The corpus lookup seam. `Err` means "no corpus could be consulted at all",
/// which the caller treats exactly like an empty result: fall back.
pub(crate) type Lookup =
    Box<dyn FnOnce(&AdvisoryPaths, &[String]) -> Result<Vec<Candidate>, String> + Send>;

/// Attach the advisory to `output`.
///
/// Non-denial outputs pass through untouched.
pub fn augment_denial(repo_root: &Path, output: HookOutput) -> HookOutput {
    let budget = configured_budget();
    if budget.is_zero() {
        return output;
    }
    augment_denial_with(
        AdvisoryPaths::for_repo(repo_root),
        output,
        budget,
        Box::new(local_corpus_lookup),
    )
}

/// Testable core: `lookup` stands in for the corpora.
///
/// Everything that can be slow or absent lives behind `lookup` and the ledger,
/// and both are bounded by `budget` as a single wall-clock allowance.
pub(crate) fn augment_denial_with(
    paths: AdvisoryPaths,
    output: HookOutput,
    budget: Duration,
    lookup: Lookup,
) -> HookOutput {
    let HookOutput::PreToolUsePermission {
        ref summary,
        ref detail,
        ..
    } = output
    else {
        return output;
    };
    let Some(signature) = block_signature(summary) else {
        return output;
    };

    let tokens: Vec<String> = signature.split(' ').map(str::to_string).collect();
    let ledger_key = signature.clone();
    let advisory = run_within(budget, move || {
        let occurrence = record_occurrence(&paths.work_notes_dir, &ledger_key);
        let entries = lookup(&paths, &tokens)
            .map(|candidates| select_entries(&paths, &tokens, candidates))
            .unwrap_or_default();
        (occurrence, entries)
    });

    // Budget exhausted, or the worker produced nothing worth saying: the
    // denial goes out exactly as the gate wrote it.
    let Some((occurrence, entries)) = advisory else {
        return output;
    };
    if entries.is_empty() {
        return output;
    }

    let section = render_section(&signature, occurrence, &entries);
    let detail = if detail.is_empty() {
        section
    } else {
        format!("{detail}\n\n{section}")
    };
    HookOutput::pre_tool_use_permission(summary.clone(), detail)
}

/// Run `work` against a wall-clock budget. `None` means the budget expired;
/// the worker is abandoned, not cancelled, and finishes on its own.
fn run_within<T: Send + 'static>(
    budget: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(budget).ok()
}

fn configured_budget() -> Duration {
    let millis = std::env::var(BUDGET_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_BUDGET_MS);
    Duration::from_millis(millis)
}

// ---------------------------------------------------------------------------
// Corpus lookup
// ---------------------------------------------------------------------------

/// Read both local corpora. `Err` only when neither could be consulted at all —
/// an empty `Ok` means "searched, found nothing", which is a different fact.
pub(crate) fn local_corpus_lookup(
    paths: &AdvisoryPaths,
    tokens: &[String],
) -> Result<Vec<Candidate>, String> {
    let memory = memory_candidates(&paths.memory_file, tokens);
    let issues = paths
        .issue_cache_root
        .as_deref()
        .and_then(|root| open_issue_candidates(root, tokens));

    if memory.is_none() && issues.is_none() {
        return Err("no work-notes memory and no Issue cache for this repository".to_string());
    }
    let mut candidates = memory.unwrap_or_default();
    candidates.extend(issues.unwrap_or_default());
    Ok(candidates)
}

/// Score every `## ` block of the work-notes memory file.
///
/// `None` distinguishes "no memory corpus exists" from "the corpus had no
/// match": only the former counts toward the unavailable-corpus fallback.
fn memory_candidates(path: &Path, tokens: &[String]) -> Option<Vec<Candidate>> {
    if fs::metadata(path).ok()?.len() > MAX_MEMORY_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;

    let mut candidates = Vec::new();
    let mut heading: Option<String> = None;
    let mut body = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(previous) = heading.take() {
                candidates.extend(memory_candidate(&previous, &body, tokens));
            }
            heading = Some(rest.trim().to_string());
            body.clear();
        } else if heading.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(previous) = heading {
        candidates.extend(memory_candidate(&previous, &body, tokens));
    }
    Some(candidates)
}

fn memory_candidate(heading: &str, body: &str, tokens: &[String]) -> Option<Candidate> {
    // `## <date> — <title>`: the date is bookkeeping, the title is the claim.
    let title = heading
        .split_once(" — ")
        .map(|(_, title)| title)
        .unwrap_or(heading)
        .trim();
    let score = overlap_score(tokens, &format!("{title}\n{body}"));
    if score < RELEVANCE_FLOOR {
        return None;
    }
    Some(Candidate {
        origin: EntryOrigin::CapturedMemory,
        title: title.to_string(),
        // The actionable half of a `memory.add` entry. An agent reading a
        // denial needs the move, not the diagnosis.
        detail: field(body, "Future Action")
            .or_else(|| field(body, "Learning"))
            .unwrap_or_default(),
        score,
    })
}

/// Read a `Name: value` field out of a memory entry body.
fn field(body: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    body.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Score the OPEN Issues in `cache_root` by title.
///
/// Titles only: bodies run to tens of megabytes across a thousand Issues, and
/// the entries that survive ranking have one body line read back in
/// [`select_entries`].
fn open_issue_candidates(cache_root: &Path, tokens: &[String]) -> Option<Vec<Candidate>> {
    let entries = fs::read_dir(cache_root).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten().take(MAX_ISSUE_SCAN) {
        let Ok(number) = entry.file_name().to_string_lossy().parse::<u64>() else {
            continue;
        };
        if crate::issue_cache::issue_is_open_in_cache(cache_root, number) != Some(true) {
            continue;
        }
        let Some(title) = crate::issue_cache::load_issue_title_from_cache(cache_root, number)
        else {
            continue;
        };
        let score = overlap_score(tokens, &title);
        if score < RELEVANCE_FLOOR {
            continue;
        }
        candidates.push(Candidate {
            origin: EntryOrigin::OpenIssue { number },
            title,
            detail: String::new(),
            score,
        });
    }
    Some(candidates)
}

/// Share of `tokens` present in `text`, `0.0..=1.0`.
fn overlap_score(tokens: &[String], text: &str) -> f64 {
    if tokens.is_empty() {
        return 0.0;
    }
    let haystack: HashSet<String> = content_tokens(text).into_iter().collect();
    let matched = tokens
        .iter()
        .filter(|token| haystack.contains(*token))
        .count();
    matched as f64 / tokens.len() as f64
}

/// Rank, cap and render the candidates.
///
/// Captured memory outranks an open Issue at equal relevance: memory records
/// what an agent actually did, an Issue records that something is still wrong.
pub(crate) fn select_entries(
    paths: &AdvisoryPaths,
    tokens: &[String],
    mut candidates: Vec<Candidate>,
) -> Vec<WorkaroundEntry> {
    candidates.retain(|candidate| candidate.score >= RELEVANCE_FLOOR);
    candidates.sort_by(|left, right| {
        let origin_rank = |candidate: &Candidate| match candidate.origin {
            EntryOrigin::CapturedMemory => 0,
            EntryOrigin::OpenIssue { .. } => 1,
        };
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| origin_rank(left).cmp(&origin_rank(right)))
    });
    candidates.truncate(MAX_ENTRIES);

    candidates
        .into_iter()
        .map(|candidate| {
            let detail = if candidate.detail.is_empty() {
                issue_body_headline(paths, &candidate, tokens).unwrap_or_default()
            } else {
                candidate.detail.clone()
            };
            WorkaroundEntry {
                origin: candidate.origin,
                title: sanitized_line(&candidate.title, SUMMARY_CHARS),
                summary: sanitized_line(&detail, SUMMARY_CHARS),
            }
        })
        .collect()
}

/// The most on-signature line of an Issue body, read only for entries that
/// already made the cut.
fn issue_body_headline(
    paths: &AdvisoryPaths,
    candidate: &Candidate,
    tokens: &[String],
) -> Option<String> {
    let EntryOrigin::OpenIssue { number } = candidate.origin else {
        return None;
    };
    let cache_root = paths.issue_cache_root.as_deref()?;
    let body = fs::read_to_string(cache_root.join(number.to_string()).join("body.md")).ok()?;
    body.lines()
        .map(str::trim)
        .filter(|line| line.len() > 20 && !line.starts_with('#') && !line.starts_with("<!--"))
        .max_by(|left, right| overlap_score(tokens, left).total_cmp(&overlap_score(tokens, right)))
        .map(str::to_string)
}

/// Collapse a hit to one redacted, control-character-free line.
///
/// Corpus text is exactly where a leaked token would be sitting, so everything
/// goes through the error-ledger sanitizer before it reaches a denial the agent
/// will echo.
fn sanitized_line(text: &str, max_chars: usize) -> String {
    let sanitized = gwt_core::error_ledger::sanitize_error_message(text);
    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max_chars {
        let mut truncated: String = collapsed.chars().take(max_chars).collect();
        truncated.push('…');
        truncated
    } else {
        collapsed
    }
}

pub(crate) fn render_section(
    signature: &str,
    occurrence: Option<u32>,
    entries: &[WorkaroundEntry],
) -> String {
    let mut section = format!("Known workarounds (block signature: `{signature}`");
    if let Some(occurrence) = occurrence {
        section.push_str(&format!(", occurrence {occurrence}"));
    }
    section.push_str("):\n");
    for entry in entries {
        match &entry.origin {
            EntryOrigin::CapturedMemory => {
                section.push_str(&format!("- work-notes memory — {}", entry.title));
            }
            EntryOrigin::OpenIssue { number } => {
                section.push_str(&format!("- open Issue #{number} — {}", entry.title));
            }
        }
        if !entry.summary.is_empty() {
            section.push_str(&format!("\n  {}", entry.summary));
        }
        section.push('\n');
    }
    section.push_str(
        "\nThese are advisory only and do not lift this gate: satisfy the Issue / Execution \
requirement stated above. If none of them applies, record what did work with `memory.add` so the \
next Execution that hits this signature is not starting over.",
    );
    section
}

// ---------------------------------------------------------------------------
// Occurrence ledger
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct SignatureLedger {
    #[serde(default)]
    signatures: BTreeMap<String, SignatureRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignatureRecord {
    occurrences: u32,
    first_seen: String,
    last_seen: String,
}

/// Count this denial against `signature` and return the running total.
///
/// `work_notes_dir` is passed in rather than derived: see [`AdvisoryPaths`].
/// `None` on any failure. The count is a detail of an advisory; losing it must
/// never cost the denial itself.
pub(crate) fn record_occurrence(work_notes_dir: &Path, signature: &str) -> Option<u32> {
    fs::create_dir_all(work_notes_dir).ok()?;

    // A dedicated lock, not the shared work-notes lock: that one is held across
    // whole memory appends and blocks indefinitely, which a hook cannot afford.
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(work_notes_dir.join("deny-signatures.lock"))
        .ok()?;
    let mut locked = false;
    for attempt in 0..3 {
        if lock.try_lock_exclusive().is_ok() {
            locked = true;
            break;
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if !locked {
        return None;
    }

    let result = bump_signature(&work_notes_dir.join("deny-signatures.json"), signature);
    let _ = FileExt::unlock(&lock);
    result
}

fn bump_signature(path: &Path, signature: &str) -> Option<u32> {
    let mut ledger: SignatureLedger = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let now = chrono::Local::now().to_rfc3339();
    let record = ledger
        .signatures
        .entry(signature.to_string())
        .or_insert_with(|| SignatureRecord {
            occurrences: 0,
            first_seen: now.clone(),
            last_seen: now.clone(),
        });
    record.occurrences = record.occurrences.saturating_add(1);
    record.last_seen = now;
    let occurrences = record.occurrences;
    let bytes = serde_json::to_vec_pretty(&ledger).ok()?;
    gwt_github::cache::write_atomic(path, &bytes).ok()?;
    Some(occurrences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    fn denial() -> HookOutput {
        HookOutput::pre_tool_use_permission(
            "Agent Workspace identity is required before work starts",
            "Run workspace.update with purpose + current_focus.",
        )
    }

    fn memory_candidate_hit(title: &str, detail: &str) -> Candidate {
        Candidate {
            origin: EntryOrigin::CapturedMemory,
            title: title.to_string(),
            detail: detail.to_string(),
            score: 1.0,
        }
    }

    /// Seed a home work-notes memory file for `repo` and return its path.
    fn seed_memory(repo: &Path, contents: &str) -> PathBuf {
        let path = gwt_core::paths::gwt_work_notes_memory_path(repo);
        fs::create_dir_all(path.parent().expect("work-notes dir")).expect("create work-notes dir");
        fs::write(&path, contents).expect("write memory");
        path
    }

    fn signature_tokens(summary: &str) -> Vec<String> {
        block_signature(summary)
            .expect("signature")
            .split(' ')
            .map(str::to_string)
            .collect()
    }

    /// AC-1: the signature is the query key, so it must survive across
    /// sessions — no paths, no issue numbers, no case differences.
    #[test]
    fn block_signature_is_stable_and_free_of_session_scoped_tokens() {
        let left = block_signature(
            "Agent Workspace identity is required before work starts (Issue #4542)",
        );
        let right =
            block_signature("agent workspace identity is required before work starts (issue #77)");
        assert_eq!(left, right, "signature must not vary with case or issue id");
        let signature = left.expect("signature");
        assert!(
            !signature.contains("4542") && !signature.contains("77"),
            "no issue id may reach the signature: {signature}"
        );
        assert_eq!(
            block_signature("Agent Workspace identity is required before work starts").as_deref(),
            Some("agent workspace identity required work starts"),
            "stopwords carry no signal and are dropped from the key"
        );
        assert_eq!(block_signature("  #1 - / the is of "), None);
    }

    /// AC-1: a hit becomes a `Known workarounds` section at the end of the
    /// denial, and the gate's own summary still leads the reason.
    #[test]
    fn denial_carries_known_workarounds_section_for_a_memory_hit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            denial(),
            Duration::from_secs(5),
            Box::new(|_paths, _tokens| {
                Ok(vec![memory_candidate_hit(
                    "workspace.update refused by the identity gate",
                    "Run execution.adopt first, then workspace.ensure.",
                )])
            }),
        );

        let reason = augmented.permission_decision_reason();
        assert!(
            reason.contains("Known workarounds"),
            "denial must carry the advisory section: {reason}"
        );
        assert!(
            reason.contains("work-notes memory — workspace.update refused by the identity gate"),
            "captured memory entry must be rendered: {reason}"
        );
        assert!(
            reason.contains("Run execution.adopt first, then workspace.ensure."),
            "the actionable half of the capture must be rendered: {reason}"
        );
        assert!(
            reason.starts_with("Agent Workspace identity is required before work starts"),
            "the gate's summary must stay the first line: {reason}"
        );
    }

    /// AC-2: no corpus, no advisory — and above all, no error. The denial is
    /// emitted exactly as the gate wrote it.
    #[test]
    fn unavailable_corpus_falls_back_to_the_unchanged_denial() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            denial(),
            Duration::from_secs(5),
            Box::new(|_paths, _tokens| Err("no corpus".to_string())),
        );

        assert_eq!(augmented, denial());
    }

    /// AC-2: and the real lookup reports exactly that when a repository has
    /// neither corpus, rather than silently answering "nothing found".
    #[test]
    fn real_lookup_reports_an_unavailable_corpus_instead_of_an_empty_result() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let paths = AdvisoryPaths {
            issue_cache_root: None,
            ..AdvisoryPaths::for_repo(temp.path())
        };
        assert!(
            local_corpus_lookup(&paths, &["identity".to_string()]).is_err(),
            "neither corpus exists yet: that is an outage, not an answer"
        );

        seed_memory(temp.path(), "# Memory\n");
        let paths = AdvisoryPaths {
            issue_cache_root: None,
            ..AdvisoryPaths::for_repo(temp.path())
        };
        assert_eq!(
            local_corpus_lookup(&paths, &["identity".to_string()]),
            Ok(Vec::new()),
            "an existing but unmatched corpus is a search, not an outage"
        );
    }

    /// AC-2: a slow lookup cannot hold the agent past the budget.
    #[test]
    fn exceeding_the_time_budget_falls_back_to_the_unchanged_denial() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let started = std::time::Instant::now();
        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            denial(),
            Duration::from_millis(50),
            Box::new(|_paths, _tokens| {
                std::thread::sleep(Duration::from_secs(5));
                Ok(vec![memory_candidate_hit("too late", "never rendered")])
            }),
        );

        assert_eq!(augmented, denial());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the denial must not wait for the abandoned lookup"
        );
    }

    /// AC-3: the same signature accumulates, and the count reaches the denial
    /// alongside the captured candidate's sanitized summary.
    #[test]
    fn occurrence_count_increments_across_denials_with_the_same_signature() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let mut reasons = Vec::new();
        for _ in 0..2 {
            let augmented = augment_denial_with(
                AdvisoryPaths::for_repo(temp.path()),
                denial(),
                Duration::from_secs(5),
                Box::new(|_paths, _tokens| {
                    Ok(vec![memory_candidate_hit("captured", "adopt then ensure")])
                }),
            );
            reasons.push(augmented.permission_decision_reason().to_string());
        }

        assert!(
            reasons[0].contains("occurrence 1"),
            "first denial must report occurrence 1: {}",
            reasons[0]
        );
        assert!(
            reasons[1].contains("occurrence 2"),
            "second denial must report occurrence 2: {}",
            reasons[1]
        );
        for reason in &reasons {
            assert!(
                reason.contains("adopt then ensure"),
                "every denial must carry the captured summary: {reason}"
            );
        }
    }

    /// AC-3: the capture the advisory surfaces is a real `memory.add` entry,
    /// read from the machine-local work-notes file, and what it shows is the
    /// `Future Action` — the move, not the diagnosis.
    #[test]
    fn a_captured_memory_entry_is_matched_and_rendered_from_its_future_action() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        seed_memory(
            temp.path(),
            "# Memory\n\n\
## 2026-09-20 — Workspace identity gate blocks work before the title is set\n\n\
Type: failure-pattern\n\
Context: A relaunched Agent hit the identity gate on its first command.\n\
Learning: The gate is lifted by workspace.ensure, not by workspace.update.\n\
Future Action: Run execution.adopt, then workspace.ensure, then workspace.update.\n\n\
## 2026-09-19 — Unrelated note about Docker volume mounts\n\n\
Type: lesson\n\
Future Action: Nothing to do with the gate.\n",
        );

        let paths = AdvisoryPaths {
            issue_cache_root: None,
            ..AdvisoryPaths::for_repo(temp.path())
        };
        let tokens = signature_tokens("Agent Workspace identity is required before work starts");

        let entries = select_entries(
            &paths,
            &tokens,
            local_corpus_lookup(&paths, &tokens).expect("lookup"),
        );

        assert_eq!(
            entries.len(),
            1,
            "only the on-signature capture: {entries:?}"
        );
        assert_eq!(entries[0].origin, EntryOrigin::CapturedMemory);
        assert_eq!(
            entries[0].title,
            "Workspace identity gate blocks work before the title is set"
        );
        assert_eq!(
            entries[0].summary,
            "Run execution.adopt, then workspace.ensure, then workspace.update."
        );
    }

    /// AC-1: only OPEN Issues are workarounds an agent can act on, and an
    /// Issue the cache cannot vouch for is not one either.
    #[test]
    fn only_open_issues_from_the_cache_reach_the_advisory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let cache_root = temp.path().join("issue-cache");

        for (number, state) in [(101u64, "open"), (102, "closed")] {
            let dir = cache_root.join(number.to_string());
            fs::create_dir_all(&dir).expect("issue dir");
            fs::write(
                dir.join("meta.json"),
                serde_json::json!({
                    "number": number,
                    "title": "Workspace identity gate blocks agent work before start",
                    "labels": [],
                    "state": state,
                    "updated_at": "2026-09-20T00:00:00Z",
                    "comment_ids": [],
                })
                .to_string(),
            )
            .expect("meta");
        }

        let tokens = signature_tokens("Agent Workspace identity is required before work starts");
        let candidates = open_issue_candidates(&cache_root, &tokens).expect("scan");

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.origin.clone())
                .collect::<Vec<_>>(),
            vec![EntryOrigin::OpenIssue { number: 101 }],
            "the closed Issue must not be offered as a workaround"
        );
    }

    /// AC-2: a weakly overlapping hit is noise. An advisory that fires on
    /// everything is one an agent learns to skip.
    #[test]
    fn weak_matches_do_not_produce_a_section() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            denial(),
            Duration::from_secs(5),
            Box::new(|_paths, _tokens| {
                Ok(vec![Candidate {
                    score: 0.2,
                    ..memory_candidate_hit("unrelated", "unrelated")
                }])
            }),
        );

        assert_eq!(augmented, denial());
    }

    /// Non-denial envelopes are not advisory surfaces.
    #[test]
    fn non_denial_outputs_pass_through_untouched() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let stop = HookOutput::stop_block("discussion is active");
        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            stop.clone(),
            Duration::from_secs(5),
            Box::new(|_paths, _tokens| Ok(vec![memory_candidate_hit("ignored", "ignored")])),
        );
        assert_eq!(augmented, stop);
    }

    /// AC-4: the section routes recovery through Issue / Execution vocabulary.
    /// The lane vocabulary the original task asked for describes machinery that
    /// PR #3429 deleted.
    #[test]
    fn advisory_section_uses_issue_and_execution_vocabulary_not_lanes() {
        let section = render_section(
            "agent workspace identity required",
            Some(2),
            &[WorkaroundEntry {
                origin: EntryOrigin::OpenIssue { number: 4542 },
                title: "deny advisory".to_string(),
                summary: "summary".to_string(),
            }],
        );
        assert!(section.contains("Issue / Execution requirement"));
        assert!(
            !section.to_lowercase().contains("lane"),
            "lane vocabulary describes machinery that no longer exists: {section}"
        );
    }

    /// Corpus text is where a leaked token would be sitting. It must not be
    /// re-emitted through the denial.
    #[test]
    fn candidate_text_is_sanitized_before_it_reaches_the_denial() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        let augmented = augment_denial_with(
            AdvisoryPaths::for_repo(temp.path()),
            denial(),
            Duration::from_secs(5),
            Box::new(|_paths, _tokens| {
                Ok(vec![memory_candidate_hit(
                    "leaky note",
                    "export GITHUB_TOKEN=ghp_exampletokenvalue0123456789",
                )])
            }),
        );

        let reason = augmented.permission_decision_reason();
        assert!(
            !reason.contains("ghp_exampletokenvalue0123456789"),
            "a secret in the corpus must not be echoed by the denial: {reason}"
        );
    }
}
