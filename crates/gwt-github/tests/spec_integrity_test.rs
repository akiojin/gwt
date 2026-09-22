//! Failure and interleaving contracts for SPEC index writes (#4613).

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

use fs2::FileExt;
use gwt_github::{
    body::{diagnose_index, Comment},
    client::{
        fake::FakeIssueClient, ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient,
        IssueCloseReason, IssueNumber, IssueSnapshot, IssueState, SpecListFilter, SpecSummary,
        UpdatedAt,
    },
    sections::SectionName,
    spec_ops::{SpecOps, SpecOpsError},
    Cache,
};
use tempfile::TempDir;

const NUMBER: IssueNumber = IssueNumber(1);
const ORIGINAL: &str = "<!-- gwt-spec id=1 version=1 -->\n<!-- sections:\nspec=body\ntasks=body\n-->\n<!-- artifact:spec BEGIN -->\nspec content\n<!-- artifact:spec END -->\n<!-- artifact:tasks BEGIN -->\nold tasks\n<!-- artifact:tasks END -->\n";

#[derive(Clone, Copy)]
enum Interference {
    FailedRollback,
    AfterCleanup,
    DuringCreation,
    MissingUnrelatedReference,
    ForeignReadback,
    None,
}

struct InterleavedClient {
    inner: FakeIssueClient,
    interference: Interference,
    patches: AtomicUsize,
    fail_fetch: AtomicBool,
    creates: AtomicUsize,
    foreign_body: Mutex<Option<String>>,
    lock_path: Option<PathBuf>,
    lock_checks: AtomicUsize,
}

impl InterleavedClient {
    fn snapshot(&self) -> IssueSnapshot {
        let FetchResult::Updated(snapshot) = self.inner.fetch(NUMBER, None).unwrap() else {
            panic!("unconditional snapshot required");
        };
        snapshot
    }

    fn check_lock(&self) {
        if let Some(path) = &self.lock_path {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .unwrap();
            let error = file
                .try_lock_exclusive()
                .expect_err("remote work must hold the OS writer lock");
            assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
            self.lock_checks.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl IssueClient for InterleavedClient {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        self.check_lock();
        if self.fail_fetch.swap(false, Ordering::SeqCst) {
            return Err(ApiError::Unexpected(
                "final fetch failed after stale-part deletion".into(),
            ));
        }
        self.inner.fetch(number, since)
    }

    fn patch_body(&self, number: IssueNumber, body: &str) -> Result<IssueSnapshot, ApiError> {
        self.check_lock();
        let attempt = self.patches.fetch_add(1, Ordering::SeqCst);
        if matches!(self.interference, Interference::FailedRollback) && attempt > 0 {
            return Err(ApiError::Unexpected("rollback PATCH rejected".into()));
        }
        let result = self.inner.patch_body(number, body)?;
        if attempt == 0 {
            let foreign = match self.interference {
                Interference::MissingUnrelatedReference => {
                    Some(body.replace("spec=body", "spec=comment:999"))
                }
                Interference::ForeignReadback => {
                    Some(format!("{body}\nConcurrent editor's independent change\n"))
                }
                _ => None,
            };
            if let Some(foreign) = foreign {
                self.inner.patch_body(number, &foreign)?;
                *self.foreign_body.lock().unwrap() = Some(foreign);
            }
        }
        Ok(result)
    }

    fn create_comment(&self, number: IssueNumber, body: &str) -> Result<CommentSnapshot, ApiError> {
        self.check_lock();
        let result = self.inner.create_comment(number, body)?;
        if self.creates.fetch_add(1, Ordering::SeqCst) == 0
            && matches!(self.interference, Interference::DuringCreation)
        {
            let foreign = ORIGINAL.replace("old tasks", "tasks changed by another writer");
            self.inner.patch_body(number, &foreign)?;
            *self.foreign_body.lock().unwrap() = Some(foreign);
        }
        Ok(result)
    }

    fn patch_title(&self, n: IssueNumber, title: &str) -> Result<IssueSnapshot, ApiError> {
        self.inner.patch_title(n, title)
    }
    fn patch_comment(&self, id: CommentId, body: &str) -> Result<CommentSnapshot, ApiError> {
        self.inner.patch_comment(id, body)
    }
    fn delete_comment(&self, id: CommentId) -> Result<(), ApiError> {
        self.inner.delete_comment(id)?;
        if matches!(self.interference, Interference::AfterCleanup) && id == CommentId(100) {
            self.fail_fetch.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        self.inner.create_issue(title, body, labels)
    }
    fn set_labels(&self, n: IssueNumber, labels: &[String]) -> Result<IssueSnapshot, ApiError> {
        self.inner.set_labels(n, labels)
    }
    fn set_state(
        &self,
        n: IssueNumber,
        state: IssueState,
        reason: Option<IssueCloseReason>,
    ) -> Result<IssueSnapshot, ApiError> {
        self.inner.set_state(n, state, reason)
    }
    fn list_spec_issues(&self, filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        self.inner.list_spec_issues(filter)
    }
}

fn fixture(interference: Interference, check_lock: bool) -> (TempDir, SpecOps<InterleavedClient>) {
    let temp = TempDir::new().unwrap();
    let inner = FakeIssueClient::new();
    inner.seed(IssueSnapshot {
        number: NUMBER,
        title: "SPEC".into(),
        body: ORIGINAL.into(),
        labels: vec!["gwt-spec".into()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("seed"),
        comments: vec![],
    });
    if matches!(
        interference,
        Interference::FailedRollback | Interference::ForeignReadback
    ) {
        inner.corrupt_next_create_comment();
    }
    if matches!(interference, Interference::AfterCleanup) {
        let FetchResult::Updated(mut snapshot) = inner.fetch(NUMBER, None).unwrap() else {
            unreachable!()
        };
        snapshot.body = ORIGINAL.replace("tasks=body", "tasks=comment:100");
        snapshot.comments.push(CommentSnapshot {
            id: CommentId(100),
            body: "<!-- artifact:tasks BEGIN -->\nold tasks\n<!-- artifact:tasks END -->".into(),
            updated_at: UpdatedAt::new("seed"),
        });
        inner.seed(snapshot);
    }
    let client = InterleavedClient {
        inner,
        interference,
        patches: AtomicUsize::new(0),
        fail_fetch: AtomicBool::new(false),
        creates: AtomicUsize::new(0),
        foreign_body: Mutex::new(None),
        lock_path: check_lock.then(|| temp.path().join(".locks/1.spec-write.lock")),
        lock_checks: AtomicUsize::new(0),
    };
    let ops = SpecOps::new(client, Cache::new(temp.path().to_owned()));
    (temp, ops)
}

fn multipart_content() -> String {
    "Task implementation and acceptance detail\n".repeat(2000)
}

fn assert_all_indexed_comments_exist(snapshot: &IssueSnapshot) {
    let comments: Vec<_> = snapshot
        .comments
        .iter()
        .map(|comment| Comment {
            id: comment.id.0,
            body: comment.body.clone(),
        })
        .collect();
    let diagnostics = diagnose_index(&snapshot.body, &comments).unwrap();
    assert!(
        !diagnostics.contains("missing:"),
        "index references deleted comments: {diagnostics}"
    );
}

#[test]
fn failed_rollback_retains_new_parts_still_referenced_by_index() {
    let (_temp, ops) = fixture(Interference::FailedRollback, false);
    let result = ops.write_section(NUMBER, &SectionName("tasks".into()), &multipart_content());
    assert!(matches!(result, Err(SpecOpsError::ReadbackMismatch { .. })));
    assert_eq!(
        ops.client().patches.load(Ordering::SeqCst),
        2,
        "must attempt rollback"
    );
    let snapshot = ops.client().snapshot();
    assert_ne!(
        snapshot.body, ORIGINAL,
        "failed rollback leaves the swapped index"
    );
    assert!(
        snapshot.comments.len() >= 2,
        "multipart comments must survive"
    );
    assert_all_indexed_comments_exist(&snapshot);
}

#[test]
fn body_changed_during_part_creation_is_not_overwritten_by_stale_swap() {
    let (_temp, ops) = fixture(Interference::DuringCreation, false);
    let result = ops.write_section(NUMBER, &SectionName("tasks".into()), &multipart_content());
    assert!(
        result.is_err(),
        "concurrent body change must refuse the stale swap"
    );
    assert_eq!(ops.client().patches.load(Ordering::SeqCst), 0);
    assert_eq!(
        ops.client().snapshot().body,
        ops.client()
            .foreign_body
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
    );
    assert!(
        ops.client().snapshot().comments.is_empty(),
        "unreferenced new parts should be cleaned up"
    );
}

#[test]
fn postwrite_missing_reference_in_other_section_refuses_verified_receipt() {
    let (_temp, ops) = fixture(Interference::MissingUnrelatedReference, false);
    let result = ops.write_section(NUMBER, &SectionName("tasks".into()), "new tasks");
    assert!(
        matches!(result, Err(SpecOpsError::IndexIntegrity(_))),
        "must validate the whole index: {result:?}"
    );
}

#[test]
fn readback_mismatch_does_not_roll_back_another_writers_body() {
    let (_temp, ops) = fixture(Interference::ForeignReadback, false);
    let result = ops.write_section(NUMBER, &SectionName("tasks".into()), &multipart_content());
    assert!(matches!(result, Err(SpecOpsError::ReadbackMismatch { .. })));
    assert_eq!(
        ops.client().patches.load(Ordering::SeqCst),
        1,
        "foreign body forbids rollback PATCH"
    );
    let snapshot = ops.client().snapshot();
    assert_eq!(
        snapshot.body,
        ops.client()
            .foreign_body
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
    );
    assert_all_indexed_comments_exist(&snapshot);
}

#[test]
fn writer_holds_host_local_lock_during_remote_reads_and_writes() {
    let (temp, ops) = fixture(Interference::None, true);
    ops.write_section(NUMBER, &SectionName("tasks".into()), &multipart_content())
        .unwrap();
    assert!(ops.client().lock_checks.load(Ordering::SeqCst) >= 4);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp.path().join(".locks/1.spec-write.lock"))
        .unwrap();
    file.try_lock_exclusive()
        .expect("writer lock must release on return");
}

#[test]
fn final_fetch_failure_never_restores_deleted_old_comment_references() {
    let (_temp, ops) = fixture(Interference::AfterCleanup, false);
    let result = ops.write_section(NUMBER, &SectionName("tasks".into()), &multipart_content());
    assert!(result.is_err(), "injected final fetch failure must surface");
    let snapshot = ops.client().snapshot();
    assert!(
        !snapshot
            .comments
            .iter()
            .any(|comment| comment.id == CommentId(100)),
        "old part was deleted before the failing fetch"
    );
    assert_all_indexed_comments_exist(&snapshot);
    assert_eq!(
        ops.client().patches.load(Ordering::SeqCst),
        1,
        "committed cleanup must not rollback the body"
    );
}
