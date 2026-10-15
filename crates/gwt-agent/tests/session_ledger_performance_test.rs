//! Issue #5393 AC-2/AC-3: same-fixture session ledger allocation measurements.
//! Counts cumulative successful allocation requests on the test thread, including
//! realloc's full requested size. These are neither live heap bytes nor RSS.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::{Duration, Instant},
};

use gwt_agent::{
    session_ledger::{load_sessions, load_sessions_shared, SessionLedgerCache},
    AgentId, Session,
};
use gwt_core::test_support::ScopedGwtHome;

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy, Default)]
struct AllocationCounts {
    requested_bytes: usize,
    calls: usize,
}

thread_local! {
    static ALLOCATIONS: Cell<Option<AllocationCounts>> = const { Cell::new(None) };
}

fn record_allocation(bytes: usize) {
    let _ = ALLOCATIONS.try_with(|counter| {
        if let Some(mut counts) = counter.get() {
            counts.requested_bytes = counts.requested_bytes.saturating_add(bytes);
            counts.calls = counts.calls.saturating_add(1);
            counter.set(Some(counts));
        }
    });
}

// SAFETY: all pointers, layouts, and sizes are forwarded unchanged to System.
// The thread-local counter does not allocate or alter allocation ownership.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller supplies a valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's valid layout is passed unchanged to System.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr and layout retain System's allocation contract.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's original allocation and new size go unchanged to System.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            record_allocation(new_size);
        }
        new_ptr
    }
}

struct Measurement {
    elapsed: Duration,
    allocations: AllocationCounts,
}

fn measure<T>(load: impl FnOnce() -> T) -> (T, Measurement) {
    ALLOCATIONS.with(|counter| counter.set(Some(AllocationCounts::default())));
    let started = Instant::now();
    let sessions = load();
    let elapsed = started.elapsed();
    let allocations =
        ALLOCATIONS.with(|counter| counter.replace(None).expect("active measurement"));
    (
        sessions,
        Measurement {
            elapsed,
            allocations,
        },
    )
}

fn print_measurement(label: &str, sessions: usize, measurement: &Measurement) {
    println!(
        "{label}: sessions={sessions}, elapsed_ms={:.3}, requested_bytes={}, allocation_calls={}",
        measurement.elapsed.as_secs_f64() * 1000.0,
        measurement.allocations.requested_bytes,
        measurement.allocations.calls,
    );
}

#[test]
fn shared_session_ledger_warm_sweeps_reduce_allocations() {
    const SESSION_COUNT: usize = 2500;

    let temp = tempfile::tempdir().expect("isolated session ledger fixture");
    let _home = ScopedGwtHome::set(temp.path());
    let sessions_dir = temp.path().join("sessions");
    std::fs::create_dir(&sessions_dir).expect("create session ledger");
    let mut template = Session::new(temp.path().join("project"), "work/fixture", AgentId::Codex);
    template.launch_command = "codex".to_string();
    template.launch_args = vec!["--fixture-prompt".to_string(), "x".repeat(21 * 1024)];
    let mut fixture_bytes = 0;
    for index in 0..SESSION_COUNT {
        template.id = format!("fixture-session-{index:04}");
        let content = toml::to_string(&template).expect("serialize fixture session");
        fixture_bytes += content.len();
        std::fs::write(sessions_dir.join(format!("{}.toml", template.id)), content)
            .expect("write fixture session");
    }
    println!(
        "fixture: sessions={SESSION_COUNT}, total_bytes={fixture_bytes}, mean_kib={:.2}",
        fixture_bytes as f64 / SESSION_COUNT as f64 / 1024.0,
    );
    println!("requested_bytes are cumulative requests, not live heap bytes or RSS");

    let (mut baseline, baseline_measurement) =
        measure(|| SessionLedgerCache::new().load(&sessions_dir));
    print_measurement(
        "single-load baseline",
        baseline.len(),
        &baseline_measurement,
    );
    assert_eq!(baseline.len(), SESSION_COUNT);
    baseline.sort_unstable_by(|left, right| left.id.cmp(&right.id));

    let mut owned_warm_requested_bytes = 0;
    for (pass, label) in ["shared cold", "shared second warm", "shared third warm"]
        .into_iter()
        .enumerate()
    {
        let (mut current, measurement) =
            measure(|| load_sessions(&sessions_dir).expect("load shared session ledger"));
        print_measurement(label, current.len(), &measurement);
        assert_eq!(current.len(), SESSION_COUNT);
        current.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        for (expected, actual) in baseline.iter().zip(&current) {
            assert_eq!(
                toml::Value::try_from(actual).expect("current session content"),
                toml::Value::try_from(expected).expect("baseline session content"),
                "{label} changed session {}",
                expected.id,
            );
        }
        if pass > 0 {
            owned_warm_requested_bytes = measurement.allocations.requested_bytes;
            assert!(
                measurement.allocations.requested_bytes
                    < baseline_measurement.allocations.requested_bytes,
                "{label} must allocate fewer bytes than a fresh single-load baseline",
            );
        }
    }

    let (mut snapshots, shared_measurement) =
        measure(|| load_sessions_shared(&sessions_dir).expect("load shared Session snapshots"));
    print_measurement("shared Arc warm", snapshots.len(), &shared_measurement);
    assert_eq!(snapshots.len(), SESSION_COUNT);
    snapshots.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    for (expected, actual) in baseline.iter().zip(&snapshots) {
        assert_eq!(
            toml::Value::try_from(actual.as_ref()).expect("shared Session content"),
            toml::Value::try_from(expected).expect("baseline Session content"),
            "shared Arc changed session {}",
            expected.id,
        );
    }
    assert!(
        shared_measurement.allocations.requested_bytes < owned_warm_requested_bytes,
        "shared Arc warm sweep must allocate fewer bytes than an owned warm sweep",
    );
}
