// Temporary Windows-only diagnostic include. Remove the product include before delivery.
// This counts requested Rust allocations, not native/JS heaps or OS commitment.
#[global_allocator]
static ISSUE_5561_ALLOCATOR: issue_5561_allocator::Counter = issue_5561_allocator::Counter;

mod issue_5561_allocator {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::UnsafeCell,
        ffi::c_void,
        fs::OpenOptions,
        io::Write,
        ptr,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    pub struct Counter;
    const FRAMES: usize = 24;
    const SLOTS: usize = 16_384;
    const TOP: usize = 80;
    const NONE: usize = usize::MAX;
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static BLOCKS: AtomicUsize = AtomicUsize::new(0);
    static REQUESTS: AtomicUsize = AtomicUsize::new(0);
    static OVERFLOW: AtomicUsize = AtomicUsize::new(0);
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static STARTED: AtomicBool = AtomicBool::new(false);
    static LOCKED: AtomicBool = AtomicBool::new(false);
    static SYMBOL_SESSION: u8 = 0;

    #[derive(Clone, Copy)]
    struct Entry {
        frames: [usize; FRAMES],
        estimated_bytes: usize,
        estimated_blocks: usize,
        sampled_blocks: usize,
        occupied: bool,
    }
    impl Entry {
        const EMPTY: Self = Self {
            frames: [0; FRAMES],
            estimated_bytes: 0,
            estimated_blocks: 0,
            sampled_blocks: 0,
            occupied: false,
        };
    }
    struct Table(UnsafeCell<[Entry; SLOTS]>);
    // All accesses are protected by LOCKED; allocation never occurs under it.
    unsafe impl Sync for Table {}
    static TABLE: Table = Table(UnsafeCell::new([Entry::EMPTY; SLOTS]));

    struct Guard;
    impl Guard {
        fn take() -> Self {
            while LOCKED
                .compare_exchange_weak(false, true, std::sync::atomic::Ordering::Acquire, Relaxed)
                .is_err()
            {
                std::hint::spin_loop();
            }
            Self
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            LOCKED.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    #[derive(Clone, Copy)]
    #[repr(C)]
    struct Header {
        slot: usize,
        weight: usize,
    }

    fn extended(layout: Layout) -> Option<(Layout, usize)> {
        let alignment = layout.align().max(std::mem::align_of::<Header>());
        let offset = std::mem::size_of::<Header>().checked_add(alignment - 1)? & !(alignment - 1);
        let full = Layout::from_size_align(offset.checked_add(layout.size())?, alignment).ok()?;
        Some((full, offset))
    }

    fn sample(size: usize) -> Header {
        let weight = if size >= 65_536 { 1 } else { 64 };
        if !ENABLED.load(Relaxed) || (weight == 64 && REQUESTS.fetch_add(1, Relaxed) % 64 != 0) {
            return Header { slot: NONE, weight };
        }
        let mut frames = [0usize; FRAMES];
        // The Windows unwinder writes into caller-owned fixed storage.
        unsafe {
            RtlCaptureStackBackTrace(
                2,
                FRAMES as u32,
                frames.as_mut_ptr().cast(),
                ptr::null_mut(),
            );
        }
        let hash = frames.iter().fold(0xcbf29ce484222325usize, |hash, frame| {
            (hash ^ frame).wrapping_mul(0x100000001b3)
        });
        let _guard = Guard::take();
        let table = unsafe { &mut *TABLE.0.get() };
        // Bound contention even after many unique allocation stacks fill the table.
        for step in 0..64 {
            let slot = hash.wrapping_add(step) % SLOTS;
            let entry = &mut table[slot];
            if !entry.occupied || entry.frames == frames {
                entry.occupied = true;
                entry.frames = frames;
                entry.estimated_bytes += size * weight;
                entry.estimated_blocks += weight;
                entry.sampled_blocks += 1;
                return Header { slot, weight };
            }
        }
        OVERFLOW.fetch_add(1, Relaxed);
        Header { slot: NONE, weight }
    }

    fn resize_sample(header: Header, old_size: usize, new_size: usize, freed: bool) {
        if header.slot == NONE {
            return;
        }
        let _guard = Guard::take();
        let entry = unsafe { &mut (*TABLE.0.get())[header.slot] };
        entry.estimated_bytes -= old_size * header.weight;
        entry.estimated_bytes += new_size * header.weight;
        if freed {
            entry.estimated_blocks -= header.weight;
            entry.sampled_blocks -= 1;
        }
    }

    unsafe impl GlobalAlloc for Counter {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { allocate(layout, false) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            unsafe { allocate(layout, true) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            let (full, offset) = extended(layout).expect("previous allocation layout");
            let base = unsafe { pointer.sub(offset) };
            let header = unsafe { ptr::read(base.cast::<Header>()) };
            LIVE.fetch_sub(layout.size(), Relaxed);
            BLOCKS.fetch_sub(1, Relaxed);
            resize_sample(header, layout.size(), 0, true);
            unsafe { System.dealloc(base, full) };
        }
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let Some((full, offset)) = extended(layout) else {
                return ptr::null_mut();
            };
            let Some(new_full_size) = offset.checked_add(new_size) else {
                return ptr::null_mut();
            };
            // System::realloc requires the resulting size to form a valid Layout.
            if Layout::from_size_align(new_full_size, full.align()).is_err() {
                return ptr::null_mut();
            }
            let base = unsafe { pointer.sub(offset) };
            let header = unsafe { ptr::read(base.cast::<Header>()) };
            let next = unsafe { System.realloc(base, full, new_full_size) };
            if next.is_null() {
                return next;
            }
            if new_size >= layout.size() {
                LIVE.fetch_add(new_size - layout.size(), Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Relaxed);
            }
            resize_sample(header, layout.size(), new_size, false);
            unsafe { next.add(offset) }
        }
    }

    unsafe fn allocate(layout: Layout, zeroed: bool) -> *mut u8 {
        let Some((full, offset)) = extended(layout) else {
            return ptr::null_mut();
        };
        let base = if zeroed {
            unsafe { System.alloc_zeroed(full) }
        } else {
            unsafe { System.alloc(full) }
        };
        if base.is_null() {
            return base;
        }
        LIVE.fetch_add(layout.size(), Relaxed);
        BLOCKS.fetch_add(1, Relaxed);
        let header = sample(layout.size());
        unsafe {
            ptr::write(base.cast::<Header>(), header);
            base.add(offset)
        }
    }

    // Raw bindings keep this diagnostic independent of Cargo feature changes.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn RtlCaptureStackBackTrace(
            skip: u32,
            count: u32,
            frames: *mut *mut c_void,
            hash: *mut u32,
        ) -> u16;
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn SymInitializeW(process: *mut c_void, path: *const u16, invade: i32) -> i32;
        fn SymLoadModuleExW(
            process: *mut c_void,
            file: *mut c_void,
            image: *const u16,
            module: *const u16,
            base: u64,
            size: u32,
            data: *const c_void,
            flags: u32,
        ) -> u64;
        fn SymFromAddr(
            process: *mut c_void,
            address: u64,
            displacement: *mut u64,
            symbol: *mut SymbolInfo,
        ) -> i32;
        fn SymCleanup(process: *mut c_void) -> i32;
    }
    #[repr(C)]
    struct SymbolInfo {
        size_of_struct: u32,
        type_index: u32,
        reserved: [u64; 2],
        index: u32,
        size: u32,
        module_base: u64,
        flags: u32,
        value: u64,
        address: u64,
        register: u32,
        scope: u32,
        tag: u32,
        name_length: u32,
        max_name_length: u32,
        name: [u8; 1],
    }
    struct Symbols {
        handle: *mut c_void,
        loaded: bool,
    }
    impl Symbols {
        fn new() -> Self {
            use std::os::windows::ffi::OsStrExt;
            // A private key plus explicit module load avoids sharing Rust's
            // backtrace symbol session. Only this reporter thread uses our key.
            let handle = ptr::addr_of!(SYMBOL_SESSION).cast_mut().cast();
            let path = std::env::current_exe().ok();
            let search = path.as_ref().and_then(|path| path.parent()).map(|path| {
                path.as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            });
            // Restrict lookup to the executable directory; do not consult an
            // ambient symbol-server configuration or download symbols.
            let initialized = search
                .as_ref()
                .is_some_and(|search| (unsafe { SymInitializeW(handle, search.as_ptr(), 0) }) != 0);
            let loaded = initialized
                && path.is_some_and(|path| {
                    let image = path
                        .as_os_str()
                        .encode_wide()
                        .chain(Some(0))
                        .collect::<Vec<_>>();
                    unsafe {
                        SymLoadModuleExW(
                            handle,
                            ptr::null_mut(),
                            image.as_ptr(),
                            ptr::null(),
                            GetModuleHandleW(ptr::null()) as u64,
                            0,
                            ptr::null(),
                            0,
                        ) != 0
                    }
                });
            Self {
                handle: if initialized { handle } else { ptr::null_mut() },
                loaded,
            }
        }
        fn resolve(&self, address: usize) -> String {
            // SYMBOL_INFO needs 8-byte alignment and trailing name storage.
            let mut storage = [0u64; 288];
            let info = storage.as_mut_ptr().cast::<SymbolInfo>();
            let mut displacement = 0;
            unsafe {
                (*info).size_of_struct = std::mem::size_of::<SymbolInfo>() as u32;
                (*info).max_name_length = 2048;
                if self.loaded
                    && SymFromAddr(self.handle, address as u64, &mut displacement, info) != 0
                {
                    let name = std::slice::from_raw_parts(
                        (*info).name.as_ptr(),
                        (*info).name_length.min(2047) as usize,
                    );
                    return format!("{}+0x{displacement:x}", String::from_utf8_lossy(name));
                }
            }
            format!("0x{address:x}")
        }
    }
    impl Drop for Symbols {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    SymCleanup(self.handle);
                }
            }
        }
    }

    pub fn start() {
        let Some(directory) = std::env::var_os("ISSUE_5561_CAPTURE_DIRECTORY") else {
            return;
        };
        if STARTED.swap(true, Relaxed) {
            return;
        }
        let directory = std::path::PathBuf::from(directory);
        if std::fs::create_dir_all(&directory).is_err() {
            return;
        }
        let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("allocator.jsonl"))
        else {
            return;
        };
        ENABLED.store(true, Relaxed);
        std::thread::Builder::new().name("issue-5561-allocation".into()).spawn(move || {
            let started = Instant::now();
            let symbols = Symbols::new();
            loop {
                let iteration_started = Instant::now();
                let elapsed = started.elapsed().as_secs_f64();
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
                let live = LIVE.load(Relaxed);
                let blocks = BLOCKS.load(Relaxed);
                let mut top = [Entry::EMPTY; TOP];
                {
                    let _guard = Guard::take();
                    for entry in unsafe { &*TABLE.0.get() }.iter() {
                        if entry.sampled_blocks == 0 { continue; }
                        let (minimum, smallest) = top.iter().enumerate().min_by_key(|(_, value)| value.estimated_bytes).unwrap();
                        if entry.estimated_bytes > smallest.estimated_bytes { top[minimum] = *entry; }
                    }
                }
                top.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.estimated_bytes));
                let stacks = top.iter().filter(|entry| entry.sampled_blocks > 0).map(|entry| {
                    let frames = entry.frames.iter().copied().take_while(|address| *address != 0)
                        .map(|address| serde_json::json!({"address":address,"symbol":symbols.resolve(address)})).collect::<Vec<_>>();
                    serde_json::json!({"estimated_live_bytes":entry.estimated_bytes,
                        "estimated_live_blocks":entry.estimated_blocks,"sampled_live_blocks":entry.sampled_blocks,"frames":frames})
                }).collect::<Vec<_>>();
                let row = serde_json::json!({
                    "schema_version":1,"pid":std::process::id(),
                    "unix_timestamp_seconds":timestamp,
                    "elapsed_seconds":elapsed,"exact_requested_live_bytes":live,"exact_live_blocks":blocks,
                    "overflow_samples":OVERFLOW.load(Relaxed),"stack_table_capacity":SLOTS,
                    "stack_table_bytes":std::mem::size_of::<Table>(),"minimum_header_bytes":std::mem::size_of::<Header>(),
                    "executable_symbols_loaded":symbols.loaded,"top_stacks":stacks,
                    "method":"Successful System allocator requests; initial >=65536 B always sampled, other initial requests 1/64 deterministic process-wide; realloc retains initial stack and sample weight; fixed table never grows or reclaims stack identities, with at most64 probes per sampled allocation; top80 weighted live stacks.",
                    "limits":"Requested Rust bytes exclude allocator headers/table, allocator/OS overhead, native/JS heaps; reporter allocations are included. Atomic total fields and table are sampled successively under concurrent allocation, not one stop-the-world instant. Sampling weights are estimates with initial-size/realloc and deterministic-sampling bias; overflow drops stack attribution but never exact totals. Unresolved frames retain addresses. Diagnostic headers/table and unwind/symbol work perturb timing and OS commitment."
                });
                if serde_json::to_writer(&mut file, &row).is_err() || file.write_all(b"\n").is_err() || file.flush().is_err() { break; }
                std::thread::sleep(Duration::from_secs(60).saturating_sub(iteration_started.elapsed()));
            }
        }).ok();
    }
}
