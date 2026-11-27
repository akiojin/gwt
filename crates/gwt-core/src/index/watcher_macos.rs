//! Single-root FSEvents backend with exclusion registered before stream startup.
//!
//! `notify` does not expose its stream, so the index watcher uses this private
//! adapter while retaining `notify-debouncer-mini` for debounce and batching.
//!
//! The stream is delivered on a private serial dispatch queue
//! (`FSEventStreamSetDispatchQueue`), the replacement Apple and
//! `objc2-core-services` name for the deprecated run-loop scheduling API.
//! lint.yml's `Clippy (macOS)` job (#4522) compiles this module, so a newly
//! deprecated call fails CI rather than develop.

use std::{
    ffi::{c_char, c_void, CStr, OsStr},
    os::unix::ffi::OsStrExt,
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use notify::{Config, Error, Event, EventHandler, EventKind, RecursiveMode, Result, WatcherKind};
use objc2_core_foundation::{CFArray, CFString};
use objc2_core_services::{
    kFSEventStreamCreateFlagFileEvents, kFSEventStreamCreateFlagNoDefer,
    kFSEventStreamEventIdSinceNow, ConstFSEventStreamRef, FSEventStreamContext,
    FSEventStreamCreate, FSEventStreamEventFlags, FSEventStreamEventId, FSEventStreamInvalidate,
    FSEventStreamRef, FSEventStreamRelease, FSEventStreamSetDispatchQueue,
    FSEventStreamSetExclusionPaths, FSEventStreamStart, FSEventStreamStop,
};

type SharedHandler = Arc<Mutex<Box<dyn EventHandler>>>;

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StartupStep {
    Excluded(Vec<PathBuf>),
    Started,
}

pub(super) struct WorktreeWatcher {
    handler: SharedHandler,
    stream: Option<NativeStream>,
}

#[cfg(test)]
impl WorktreeWatcher {
    pub(super) fn startup_trace(&self) -> Vec<StartupStep> {
        self.stream.as_ref().unwrap().trace.clone()
    }
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
        // On any failure, dropping the partial stream tears down what started.
        self.stream = Some(NativeStream::start(
            root,
            &root_string,
            &excluded_string,
            self.handler.clone(),
        )?);
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

struct CallbackContext {
    handler: SharedHandler,
}

struct NativeStream {
    root: PathBuf,
    raw: FSEventStreamRef,
    queue: DispatchRetained<DispatchQueue>,
    queued: bool,
    started: bool,
    // Declared after the lifecycle state: Drop releases the stream and drains
    // the queue before this allocation is freed.
    _context: Box<CallbackContext>,
    #[cfg(test)]
    trace: Vec<StartupStep>,
}

// SAFETY: the stream ref is owned exclusively by this value and only touched
// through `&mut self` or Drop. Streams delivered on a dispatch queue accept
// lifecycle calls from any thread; callbacks run on that serial queue.
unsafe impl Send for NativeStream {}

impl NativeStream {
    fn start(
        root: PathBuf,
        root_path: &str,
        excluded_path: &str,
        handler: SharedHandler,
    ) -> Result<Self> {
        // This stable allocation outlives the stream, including its Release.
        let mut context = Box::new(CallbackContext { handler });
        let mut native_context = FSEventStreamContext {
            version: 0,
            info: (&mut *context as *mut CallbackContext).cast(),
            retain: None,
            release: None,
            copyDescription: None,
        };
        let paths = CFArray::from_objects(&[&*CFString::from_str(root_path)]);
        let exclusions = CFArray::from_objects(&[&*CFString::from_str(excluded_path)]);
        // SAFETY: context outlives the stream, and Create copies the path array.
        // FileEvents without UseCFTypes supplies a C-string path array to callback.
        let raw = unsafe {
            FSEventStreamCreate(
                None,
                Some(callback),
                &mut native_context,
                paths.as_opaque(),
                kFSEventStreamEventIdSinceNow,
                0.05,
                kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer,
            )
        };
        if raw.is_null() {
            return Err(Error::generic("FSEventStreamCreate failed"));
        }
        let mut stream = Self {
            root,
            raw,
            queue: DispatchQueue::new("gwt index FSEvents", DispatchQueueAttr::SERIAL),
            queued: false,
            started: false,
            _context: context,
            #[cfg(test)]
            trace: Vec::new(),
        };
        // SAFETY: raw is valid and has not started. An exclusion may name a target
        // directory that does not exist yet. Failure must not leave an unexcluded watch.
        if !unsafe { FSEventStreamSetExclusionPaths(raw, exclusions.as_opaque()) } {
            return Err(Error::generic("FSEvents target exclusion failed"));
        }
        #[cfg(test)]
        stream.trace.push(StartupStep::Excluded(
            exclusions
                .iter()
                .map(|path| PathBuf::from(path.to_string()))
                .collect(),
        ));
        // SAFETY: raw is valid; the queue is retained by the stream until Invalidate.
        unsafe { FSEventStreamSetDispatchQueue(raw, Some(&stream.queue)) };
        stream.queued = true;
        // SAFETY: the valid stream has a dispatch queue.
        if !unsafe { FSEventStreamStart(raw) } {
            return Err(Error::generic("FSEventStreamStart failed"));
        }
        stream.started = true;
        #[cfg(test)]
        stream.trace.push(StartupStep::Started);
        Ok(stream)
    }
}

impl Drop for NativeStream {
    fn drop(&mut self) {
        let raw = self.raw;
        // SAFETY: raw came from a successful Create. Each lifecycle call is
        // paired only with its corresponding successful earlier transition.
        unsafe {
            if self.started {
                FSEventStreamStop(raw);
            }
            if self.queued {
                FSEventStreamInvalidate(raw);
            }
        }
        if self.queued {
            // A callback already running on the serial queue finishes before
            // the stream and its context are freed.
            self.queue.exec_sync(|| {});
        }
        // SAFETY: no callback can run any more; this releases Create's reference.
        unsafe { FSEventStreamRelease(raw) };
    }
}

unsafe extern "C-unwind" fn callback(
    _stream: ConstFSEventStreamRef,
    info: *mut c_void,
    count: usize,
    paths: NonNull<c_void>,
    _flags: NonNull<FSEventStreamEventFlags>,
    _ids: NonNull<FSEventStreamEventId>,
) {
    // Rust unwinding must never cross the native callback boundary.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if count == 0 {
            return;
        }
        // SAFETY: FSEvents supplies count valid C strings for this callback's
        // lifetime, and info points to our Box retained until stream Release.
        let context = unsafe { &*info.cast::<CallbackContext>() };
        let paths =
            unsafe { std::slice::from_raw_parts(paths.as_ptr().cast::<*const c_char>(), count) };
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
