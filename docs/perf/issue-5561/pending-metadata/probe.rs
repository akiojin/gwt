//! Phase-local allocation probe; measure.ps1 supplies the actual product types.
use gwt_core::{
    work_events_intake::work_events_intake_group_of,
    workspace_projection::WorkspaceExecutionContainerRef,
};
use sha2::{Digest, Sha256};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashSet,
    sync::{
        atomic::{AtomicUsize, Ordering::Relaxed},
        Arc,
    },
};

const SOURCE_WORKTREE: &str = "worktree:";
fn source_group(key: &str) -> &str {
    work_events_intake_group_of(key).unwrap_or(SOURCE_WORKTREE)
}
include!(env!("ISSUE_5561_PRODUCT_TYPES"));

struct Counter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: Counter = Counter;
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(live, Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            let live = LIVE.fetch_add(size, Relaxed) + size - layout.size();
            LIVE.fetch_sub(layout.size(), Relaxed);
            PEAK.fetch_max(live, Relaxed);
        }
        next
    }
}

fn main() {
    let groups = 24;
    let shards = 8000;
    let baseline = LIVE.load(Relaxed);
    PEAK.store(baseline, Relaxed);
    let content: Arc<str> = Arc::from("");
    let mut sources = Vec::with_capacity(groups * shards);
    #[cfg(not(before))]
    let mut parts = HashSet::new();
    let mut expected = Sha256::new();
    for group in 0..groups {
        for shard in 0..shards {
            let suffix = format!(".gwt/work/events/{:02x}/{:064x}.jsonl", shard % 256, shard);
            let key = if group % 2 == 0 {
                format!("worktree:C:/fixture/work/issue-{group}/{suffix}")
            } else {
                format!("ref:refs/remotes/origin/work/issue-{group}:{suffix}")
            };
            let fingerprint = hex::encode(Sha256::digest(key.as_bytes()));
            expected.update(key.as_bytes());
            expected.update(fingerprint.as_bytes());
            sources.push(PendingWorkEventsSource {
                #[cfg(before)]
                key: key.clone(),
                #[cfg(not(before))]
                key: SourceKey::capture(&key, &mut parts),
                #[cfg(before)]
                fingerprint: fingerprint.clone(),
                #[cfg(not(before))]
                fingerprint: compact_source_fingerprint(&fingerprint),
                content: Arc::clone(&content),
                container: None,
                reload_from_worktree: true,
            });
        }
    }
    let retained = LIVE.load(Relaxed) - baseline;
    let peak = PEAK.load(Relaxed) - baseline;
    let mut actual = Sha256::new();
    for source in &sources {
        #[cfg(before)]
        let (key, fingerprint) = (source.key.clone(), source.fingerprint.clone());
        #[cfg(not(before))]
        let (key, fingerprint) = (
            source.key.restore(),
            restore_source_fingerprint(&source.fingerprint),
        );
        actual.update(key.as_bytes());
        actual.update(fingerprint.as_bytes());
    }
    let expected = hex::encode(expected.finalize());
    let actual = hex::encode(actual.finalize());
    assert_eq!(
        actual, expected,
        "all persisted identities must be unchanged"
    );
    drop(sources);
    #[cfg(not(before))]
    drop(parts);
    drop(content);
    // Digest strings still live here; report them separately from pending state.
    let after_drop = LIVE.load(Relaxed) - baseline;
    println!("{{\"groups\":{groups},\"shards\":{shards},\"sources\":{},\"retained_bytes\":{retained},\"peak_bytes\":{peak},\"after_drop_bytes\":{after_drop},\"identity_sha256\":\"{actual}\",\"round_trip_equal\":true}}", groups * shards);
}
