//! Contract tests for the cargo cache warm workflow (Issue #4133).
//!
//! `Swatinem/rust-cache` only ever writes into the scope of the branch that ran
//! it. `test.yml` / `build.yml` / `lint.yml` fire on `pull_request` alone, so
//! nothing populated the develop or main scope and every PR's first Rust job
//! recorded `No cache found.`, rebuilt 377 crates instead of 13, and threw the
//! 540 MB it saved away when the PR closed (~52 machine-min per PR).
//!
//! The fix is a push-triggered warm workflow, and its only load-bearing
//! property is that it computes **the same cache key** as the PR jobs. That key
//! is `{prefix-key}-{shared-key or job id}-{os}-{arch}-{toolchain hash}-{lock
//! hash}`, so a warm job that drifts on the shared key, on `runs-on`, or on the
//! toolchain action produces a cache nobody restores — a silent regression back
//! to today's behaviour that no CI run reports as a failure. These tests pin
//! each of those inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde_yaml::Value;

const WARM_WORKFLOW: &str = ".github/workflows/cache-warm.yml";
const PR_WORKFLOWS: [&str; 3] = [
    ".github/workflows/test.yml",
    ".github/workflows/build.yml",
    ".github/workflows/lint.yml",
];
const RUST_CACHE_ACTION: &str = "Swatinem/rust-cache";
const TOOLCHAIN_ACTION: &str = "dtolnay/rust-toolchain@stable";

/// AC-3: the repository cache ceiling is 10 GB and one warmed scope measured
/// ~540 MB. Each shared key is stored once per warmed branch scope (develop and
/// main), so the shared keys alone cost `count * 2 * 540 MB`. Three keys leave
/// ~3.2 GB for `coverage.yml` and the release build scopes, which keep their
/// own job-id keys. A fourth key would have to be justified against that
/// budget rather than added by habit.
const MAX_SHARED_KEYS: usize = 3;

/// A `Swatinem/rust-cache` use site: which job it belongs to, the runner it
/// runs on, and the cache-scope inputs it declares.
#[derive(Debug)]
struct CacheSite {
    workflow: String,
    job: String,
    runs_on: String,
    shared_key: Option<String>,
    key: Option<String>,
    uses_stable_toolchain: bool,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("gwt crate must be nested under crates/")
        .to_path_buf()
}

fn read_workflow(relative: &str) -> Value {
    let path = repo_root().join(relative);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_yaml::from_str(&raw).unwrap_or_else(|error| panic!("parse {relative}: {error}"))
}

/// YAML resolves a bare `on:` key differently across parsers (1.1 reads it as
/// `true`), so both spellings are accepted.
fn triggers(doc: &Value) -> &Value {
    doc.get("on")
        .or_else(|| doc.get(Value::Bool(true)))
        .unwrap_or_else(|| panic!("workflow must declare triggers"))
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(single)) => vec![single.clone()],
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Every `Swatinem/rust-cache` step declared by `relative`, in job order.
fn cache_sites(relative: &str) -> Vec<CacheSite> {
    let doc = read_workflow(relative);
    let jobs = doc
        .get("jobs")
        .and_then(Value::as_mapping)
        .unwrap_or_else(|| panic!("{relative} must declare jobs"));

    let mut sites = Vec::new();
    for (job_id, job) in jobs {
        let job_id = job_id.as_str().unwrap_or_default().to_string();
        let runs_on = job
            .get("runs-on")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let steps = job
            .get("steps")
            .and_then(Value::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let uses_stable_toolchain = steps
            .iter()
            .any(|step| step.get("uses").and_then(Value::as_str) == Some(TOOLCHAIN_ACTION));
        for step in steps {
            let uses = step.get("uses").and_then(Value::as_str).unwrap_or_default();
            if !uses.starts_with(RUST_CACHE_ACTION) {
                continue;
            }
            let with = step.get("with");
            sites.push(CacheSite {
                workflow: relative.to_string(),
                job: job_id.clone(),
                runs_on: runs_on.clone(),
                shared_key: with
                    .and_then(|with| with.get("shared-key"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                key: with
                    .and_then(|with| with.get("key"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                uses_stable_toolchain,
            });
        }
    }
    sites
}

fn pr_cache_sites() -> Vec<CacheSite> {
    PR_WORKFLOWS.iter().flat_map(|w| cache_sites(w)).collect()
}

/// AC-1: a PR job's cache scope is only reproducible from another workflow if
/// it is named. Left to the default the key embeds `GITHUB_JOB`, which no warm
/// job can share without duplicating the job id, and a `key:` input alongside
/// it would reintroduce a per-job segment the moment `shared-key` is dropped.
#[test]
fn pr_rust_cache_steps_name_their_scope_with_a_shared_key() {
    let sites = pr_cache_sites();
    assert!(
        !sites.is_empty(),
        "expected {PR_WORKFLOWS:?} to use {RUST_CACHE_ACTION}"
    );
    for site in &sites {
        assert!(
            site.shared_key.as_deref().is_some_and(|k| !k.is_empty()),
            "{} job `{}` must declare a `shared-key` so the warm workflow can \
             reproduce its cache key (Issue #4133 AC-1)",
            site.workflow,
            site.job
        );
        assert!(
            site.key.is_none(),
            "{} job `{}` must not set `key:` alongside `shared-key:`; the extra \
             segment reappears in the cache key as soon as `shared-key` is \
             removed and silently unshares the scope",
            site.workflow,
            site.job
        );
    }
}

/// AC-1: every scope a PR job restores is written by a warm job on the same
/// runner with the same toolchain action, so the two sides compute an
/// identical key rather than two keys that merely look alike.
#[test]
fn every_pr_cache_scope_is_warmed_by_a_matching_job() {
    let warmed: BTreeSet<(String, String)> = cache_sites(WARM_WORKFLOW)
        .into_iter()
        .filter(|site| {
            assert!(
                site.uses_stable_toolchain,
                "{WARM_WORKFLOW} job `{}` must install the toolchain with \
                 `{TOOLCHAIN_ACTION}`; the rustc version is hashed into the \
                 cache key",
                site.job
            );
            site.shared_key.is_some()
        })
        .map(|site| (site.shared_key.unwrap_or_default(), site.runs_on))
        .collect();

    for site in pr_cache_sites() {
        let shared_key = site.shared_key.clone().unwrap_or_default();
        assert!(
            site.uses_stable_toolchain,
            "{} job `{}` must install the toolchain with `{TOOLCHAIN_ACTION}` \
             so its key matches the warm job's",
            site.workflow, site.job
        );
        assert!(
            warmed.contains(&(shared_key.clone(), site.runs_on.clone())),
            "{} job `{}` restores cache scope `{shared_key}` on `{}`, but \
             {WARM_WORKFLOW} warms none. Warmed scopes: {warmed:?}",
            site.workflow,
            site.job,
            site.runs_on
        );
    }
}

/// AC-1 / AC-4: the warm workflow has to run where PRs can restore from — the
/// develop and main scopes — and it must also re-run without a repository
/// change, because a stable-toolchain bump changes the key on its own and an
/// untouched cache is evicted after 7 days.
#[test]
fn warm_workflow_covers_pr_base_branches_and_toolchain_drift() {
    let doc = read_workflow(WARM_WORKFLOW);
    let on = triggers(&doc);

    let push_branches = string_list(on.get("push").and_then(|push| push.get("branches")));
    for branch in ["develop", "main"] {
        assert!(
            push_branches.iter().any(|b| b == branch),
            "{WARM_WORKFLOW} must warm the `{branch}` scope on push; PR runs \
             restore from their base branch and the default branch only \
             (declared: {push_branches:?})"
        );
    }

    assert!(
        on.get("schedule").is_some(),
        "{WARM_WORKFLOW} must keep a schedule: a stable-toolchain update \
         changes the cache key with no commit to trigger a push, and an \
         unrestored cache is evicted after 7 days (Issue #4133 AC-4)"
    );
    assert!(
        on.get("workflow_dispatch").is_some(),
        "{WARM_WORKFLOW} must stay manually dispatchable so a cold scope can be \
         rewarmed without waiting for the schedule"
    );
}

/// AC-3: the repository cache ceiling is 10 GB. Each shared key is stored once
/// per warmed branch scope, so the number of distinct keys is the budget knob.
#[test]
fn shared_cache_scopes_stay_within_the_repository_cache_budget() {
    let mut per_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for site in pr_cache_sites() {
        if let Some(shared_key) = site.shared_key {
            per_key.entry(shared_key).or_default().insert(site.job);
        }
    }
    assert!(
        per_key.len() <= MAX_SHARED_KEYS,
        "{} distinct shared cache scopes exceed the {MAX_SHARED_KEYS}-scope \
         budget (~540 MB each, stored for both develop and main, against a \
         10 GB repository ceiling): {per_key:?}",
        per_key.len()
    );
}
