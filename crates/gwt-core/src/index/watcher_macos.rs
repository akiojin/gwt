//! Single-root FSEvents backend with exclusion registered before stream startup.
//!
//! `notify` does not expose its stream, so the index watcher uses this private
//! adapter while retaining `notify-debouncer-mini` for debounce and batching.

use std::{
    ffi::{c_char, c_void, CStr, OsStr},
    os::unix::ffi::OsStrExt,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use core_foundation::{
    array::CFArray,
    base::TCFType,
    runloop::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRunResult},
    string::CFString,
};
use fsevent_sys as fs;
use notify::{Config, Error, Event, EventHandler, EventKind, RecursiveMode, Result, WatcherKind};

type SharedHandler = Arc<Mutex<Box<dyn EventHandler>>>;

pub(super) struct WorktreeWatcher {
    handler: SharedHandler,
    stream: Option<StreamThread>,
}

impl notify::Watcher for WorktreeWatcher {
    fn new<F: EventHandler>(handler: F, _config: Config) -> Result<Self> {
        Ok(Self {
            handler: Arc::new(Mutex::new(Box::new(handler))),
            stream: None,
        })
    }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> Result<()> {
        if self.stream.is_some() || !matches!(mode, RecursiveMode::Recursive) {
            return Err(Error::generic(
                "index watcher requires one recursive worktree root",
            ));
        }
        let root = path.canonicalize()?;
        if !root.is_dir() {
            return Err(Error::generic("worktree root must be a directory").add_path(root));
        }
        let root_string = root
            .to_str()
            .ok_or_else(|| Error::generic("FSEvents root must be valid UTF-8"))?
            .to_owned();
        let excluded = root.join("target");
        let excluded_string = excluded
            .to_str()
            .ok_or_else(|| Error::generic("FSEvents exclusion must be valid UTF-8"))?
            .to_owned();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let handler = self.handler.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("gwt index FSEvents".into())
            .spawn(move || {
                run_stream(root_string, excluded_string, handler, worker_stop, ready_tx)
            })?;
        let stream = StreamThread {
            root,
            stop,
            worker: Some(worker),
        };
        // On either failure, dropping the local stream stops and joins its worker.
        ready_rx
            .recv()
            .map_err(|_| Error::generic("FSEvents worker exited during startup"))??;
        self.stream = Some(stream);
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> Result<()> {
        if self
            .stream
            .as_ref()
            .is_none_or(|stream| stream.root != path)
        {
            return Err(Error::watch_not_found().add_path(path.to_path_buf()));
        }
        self.stream = None;
        Ok(())
    }

    fn kind() -> WatcherKind {
        WatcherKind::Fsevent
    }
}

struct StreamThread {
    root: PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for StreamThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Created and destroyed on the worker thread; no native pointer crosses threads.
struct NativeStream {
    raw: fs::FSEventStreamRef,
    scheduled: bool,
    started: bool,
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        // SAFETY: raw came from a successful Create. Each lifecycle call is
        // paired only with its corresponding successful earlier transition.
        unsafe {
            if self.started {
                fs::FSEventStreamStop(self.raw);
            }
            if self.scheduled {
                fs::FSEventStreamInvalidate(self.raw);
            }
            fs::FSEventStreamRelease(self.raw);
        }
    }
}

struct CallbackContext {
    handler: SharedHandler,
}

fn run_stream(
    root: String,
    excluded: String,
    handler: SharedHandler,
    stop: Arc<AtomicBool>,
    ready: mpsc::SyncSender<Result<()>>,
) {
    // This stable allocation outlives NativeStream, including its Release.
    let mut context = Box::new(CallbackContext { handler });
    let native_context = fs::FSEventStreamContext {
        version: 0,
        info: (&mut *context as *mut CallbackContext).cast(),
        retain: None,
        release: None,
        copy_description: None,
    };
    let paths = CFArray::from_CFTypes(&[CFString::new(&root)]);
    let exclusions = CFArray::from_CFTypes(&[CFString::new(&excluded)]);
    // SAFETY: context and both CF arrays remain alive through stream teardown.
    // FileEvents without UseCFTypes supplies a C-string path array to callback.
    let raw = unsafe {
        fs::FSEventStreamCreate(
            std::ptr::null_mut(),
            callback,
            &native_context,
            paths.as_concrete_TypeRef().cast_mut().cast(),
            fs::kFSEventStreamEventIdSinceNow,
            0.05,
            fs::kFSEventStreamCreateFlagFileEvents | fs::kFSEventStreamCreateFlagNoDefer,
        )
    };
    if raw.is_null() {
        let _ = ready.send(Err(Error::generic("FSEventStreamCreate failed")));
        return;
    }
    let mut stream = NativeStream {
        raw,
        scheduled: false,
        started: false,
    };
    // SAFETY: raw is valid and has not started. An exclusion may name a target
    // directory that does not exist yet. Failure must not leave an unexcluded watch.
    if unsafe {
        fs::FSEventStreamSetExclusionPaths(raw, exclusions.as_concrete_TypeRef().cast_mut().cast())
    } == 0
    {
        let _ = ready.send(Err(Error::generic("FSEvents target exclusion failed")));
        return;
    }
    let run_loop = CFRunLoop::get_current();
    // SAFETY: this is the worker's run loop and the system default mode constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    unsafe {
        fs::FSEventStreamScheduleWithRunLoop(
            raw,
            run_loop.as_concrete_TypeRef().cast(),
            mode.cast_mut().cast(),
        );
    }
    stream.scheduled = true;
    // SAFETY: the valid stream is scheduled on this worker's run loop.
    if unsafe { fs::FSEventStreamStart(raw) } == 0 {
        let _ = ready.send(Err(Error::generic("FSEventStreamStart failed")));
        return;
    }
    stream.started = true;
    if ready.send(Ok(())).is_err() {
        return;
    }
    while !stop.load(Ordering::Acquire) {
        // A bounded run avoids the startup race of Stop arriving before Run.
        match CFRunLoop::run_in_mode(mode, Duration::from_millis(100), false) {
            CFRunLoopRunResult::Finished | CFRunLoopRunResult::Stopped => break,
            _ => {}
        }
    }
}

extern "C" fn callback(
    _stream: fs::FSEventStreamRef,
    info: *mut c_void,
    count: usize,
    paths: *mut c_void,
    _flags: *const fs::FSEventStreamEventFlags,
    _ids: *const fs::FSEventStreamEventId,
) {
    // Rust unwinding must never cross the native callback boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if count == 0 {
            return;
        }
        // SAFETY: FSEvents supplies count valid C strings for this callback's
        // lifetime, and info points to our Box retained until stream Release.
        let context = unsafe { &*info.cast::<CallbackContext>() };
        let paths = unsafe { std::slice::from_raw_parts(paths.cast::<*const c_char>(), count) };
        let mut handler = context
            .handler
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        for &path in paths {
            let bytes = unsafe { CStr::from_ptr(path) }.to_bytes();
            let path = PathBuf::from(OsStr::from_bytes(bytes));
            handler.handle_event(Ok(Event::new(EventKind::Any).add_path(path)));
        }
    }));
}
