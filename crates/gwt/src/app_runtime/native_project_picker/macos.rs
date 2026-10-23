//! Parentless, modeless panels keep the tray's main event loop available.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    time::Instant,
};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{rc::Retained, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSModalResponse, NSModalResponseCancel,
    NSModalResponseOK, NSOpenPanel,
};

type PickerResult = Result<Option<PathBuf>, String>;

struct ActivePanel {
    panel: Retained<NSOpenPanel>,
    deadline: Instant,
    result: Sender<PickerResult>,
}

#[derive(Default)]
struct Panels {
    active: HashMap<u64, ActivePanel>,
    original_policy: Option<NSApplicationActivationPolicy>,
}

thread_local! {
    // Accessed only by main-queue closures and AppKit completion callbacks.
    static PANELS: RefCell<Panels> = RefCell::new(Panels::default());
}

fn finish(id: u64, response: NSModalResponse) {
    let mtm = MainThreadMarker::new().expect("panel completion must run on the main thread");
    let (active, restore_policy) = PANELS.with(|panels| {
        let mut panels = panels.borrow_mut();
        let active = panels.active.remove(&id);
        let restore = if panels.active.is_empty() {
            panels.original_policy.take()
        } else {
            None
        };
        (active, restore)
    });
    let Some(active) = active else { return };
    let result = if Instant::now() >= active.deadline {
        Err("Folder selection timed out".into())
    } else if response == NSModalResponseOK {
        active
            .panel
            .URL()
            .and_then(|url| url.path())
            .map(|path| Some(PathBuf::from(path.to_string())))
            .ok_or_else(|| "Folder selection returned no path".into())
    } else if response == NSModalResponseCancel {
        Ok(None)
    } else {
        Err("Cannot display folder selection".into())
    };
    active.panel.orderOut(None);
    if let Some(policy) = restore_policy {
        NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
    }
    let _ = active.result.send(result);
}

fn show(id: u64, deadline: Instant, result: Sender<PickerResult>) {
    // A request may have expired while queued behind other main-thread work.
    if Instant::now() >= deadline {
        let _ = result.send(Err("Folder selection timed out".into()));
        return;
    }
    let mtm = MainThreadMarker::new().expect("panel presentation must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let first = PANELS.with(|panels| panels.borrow().active.is_empty());
    if first {
        let policy = app.activationPolicy();
        if policy == NSApplicationActivationPolicy::Prohibited
            && !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory)
        {
            let _ = result.send(Err("Cannot activate folder selection".into()));
            return;
        }
        PANELS.with(|panels| panels.borrow_mut().original_policy = Some(policy));
    }
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    PANELS.with(|panels| {
        panels.borrow_mut().active.insert(
            id,
            ActivePanel {
                panel: panel.clone(),
                deadline,
                result,
            },
        );
    });
    let completion = RcBlock::new(move |response| finish(id, response));
    panel.beginWithCompletionHandler(&completion);
    // Display failure can complete synchronously; do not show that panel again.
    if !PANELS.with(|panels| panels.borrow().active.contains_key(&id)) {
        return;
    }
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    panel.makeKeyAndOrderFront(None);
}

pub(super) fn pick(deadline: Instant) -> PickerResult {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (send, receive) = mpsc::channel();
    DispatchQueue::main().exec_async(move || show(id, deadline, send));
    match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("Folder selection worker stopped".into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            DispatchQueue::main().exec_async(move || {
                let panel = PANELS.with(|panels| {
                    panels
                        .borrow()
                        .active
                        .get(&id)
                        .map(|active| active.panel.clone())
                });
                if let Some(panel) = panel {
                    // No RefCell borrow spans this call: cancellation may invoke
                    // the completion callback synchronously on this same thread.
                    unsafe { panel.cancel(None) };
                }
            });
            // Preserve the busy guard until AppKit has actually ended the panel.
            receive
                .recv()
                .unwrap_or_else(|_| Err("Folder selection worker stopped".into()))
        }
    }
}
