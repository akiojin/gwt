use gwt_core::work_events_intake::WorkEventsIntakeStore;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
    TOTAL.fetch_add(bytes, Ordering::Relaxed);
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            allocated(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let out = System.realloc(ptr, layout, size);
        if !out.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            allocated(size);
        }
        out
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() {
    let path = std::env::args().nth(1).expect("isolated intake path");
    let mut store = WorkEventsIntakeStore::open(std::path::Path::new(&path));
    assert_eq!(store.source_count(), 0);
    let mut expected = DefaultHasher::new();
    let started = Instant::now();
    for group_index in 0..24 {
        let group = format!("ref:refs/remotes/origin/issue-{group_index:04}");
        let mut sources = std::collections::BTreeMap::new();
        for shard in 0..8000 {
            let key = format!(
                "{group}:.gwt/work/events/{:02x}/{shard:064x}.jsonl",
                shard % 256
            );
            let value = format!("{:064x}", group_index * 8000 + shard);
            sources.insert(key, value);
        }
        for (key, value) in &sources {
            key.hash(&mut expected);
            value.hash(&mut expected);
        }
        store.set_group(&group, Some(format!("snapshot-{group_index}")), sources);
    }
    let before = LIVE.load(Ordering::Relaxed);
    let report = store.save().expect("save");
    let after = LIVE.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    let total = TOTAL.load(Ordering::Relaxed);
    let elapsed_ms = started.elapsed().as_millis();
    assert_eq!(report.groups_written, 24);
    assert_eq!(store.source_count(), 192000);
    let mut actual = DefaultHasher::new();
    let state = store.to_state().expect("reload exact saved values");
    for (key, value) in &state.sources {
        key.hash(&mut actual);
        value.hash(&mut actual);
    }
    assert_eq!(actual.finish(), expected.finish());
    println!("{{\"groups\":24,\"sources\":192000,\"before_save_live_bytes\":{before},\"after_save_live_bytes\":{after},\"peak_live_bytes\":{peak},\"total_requested_bytes\":{total},\"elapsed_ms\":{elapsed_ms},\"persisted_values_digest\":\"{:016x}\",\"groups_written\":{},\"bytes_written\":{}}}", actual.finish(), report.groups_written, report.bytes_written);
    std::hint::black_box(store);
}
