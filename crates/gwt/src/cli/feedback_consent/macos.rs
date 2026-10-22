use std::cell::Cell;

use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, sel, DefinedClass,
    MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBezelStyle, NSButton,
    NSScreen, NSScrollView, NSTextView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use super::{bounded_window_size, DECLINE_LABEL, FOOTER_HEIGHT, PADDING, SEND_LABEL, TITLE};

define_class!(
    // SAFETY: NSObject imposes no subclass requirements. The class only runs
    // on the main thread, and its target/delegate is retained throughout modal.
    #[unsafe(super = NSObject)]
    #[name = "GWTFeedbackConsentDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<bool>]
    struct ConsentDelegate;

    unsafe impl NSObjectProtocol for ConsentDelegate {}

    impl ConsentDelegate {
        #[unsafe(method(sendFeedback:))]
        fn send_feedback(&self, _sender: &NSButton) {
            self.ivars().set(true);
            NSApplication::sharedApplication(self.mtm()).stopModalWithCode(1);
        }

        #[unsafe(method(declineFeedback:))]
        fn decline_feedback(&self, _sender: &NSButton) {
            self.ivars().set(false);
            NSApplication::sharedApplication(self.mtm()).stopModalWithCode(0);
        }
    }

    unsafe impl NSWindowDelegate for ConsentDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _window: &NSWindow) -> bool {
            self.ivars().set(false);
            NSApplication::sharedApplication(self.mtm()).stopModalWithCode(0);
            true
        }
    }
);

pub(super) fn confirm(prompt: &str) -> Result<bool, String> {
    let mtm = MainThreadMarker::new()
        .ok_or("Native feedback consent must run on the worker's main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
        return Err("Cannot activate the native feedback consent application".into());
    }
    // This worker has no NSApplication::run loop. Complete application launch
    // before creating a custom window so LaunchServices observes a ready app.
    app.finishLaunching();
    let screen = NSScreen::mainScreen(mtm).ok_or("No desktop screen is available")?;
    let work = screen.visibleFrame();
    let size = bounded_window_size(work.size.width as i32, work.size.height as i32)?;
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    let outer = NSRect::new(
        NSPoint::new(
            work.origin.x + (work.size.width - f64::from(size.width)) / 2.0,
            work.origin.y + (work.size.height - f64::from(size.height)) / 2.0,
        ),
        NSSize::new(f64::from(size.width), f64::from(size.height)),
    );
    let content = NSWindow::contentRectForFrameRect_styleMask(outer, style, mtm);
    // SAFETY: window ownership remains in this function and AppKit's automatic
    // release-on-close is disabled immediately after construction.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            content,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(TITLE));
    let view = window
        .contentView()
        .ok_or("Feedback window has no content view")?;
    let width = content.size.width;
    let body_height = content.size.height - f64::from(FOOTER_HEIGHT + PADDING);
    let body_width = width - f64::from(2 * PADDING);
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(
            NSPoint::new(f64::from(PADDING), f64::from(FOOTER_HEIGHT)),
            NSSize::new(body_width, body_height),
        ),
    );
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(false);
    scroll.setAutohidesScrollers(false);
    let text_size = scroll.contentSize();
    let text = NSTextView::initWithFrame(
        NSTextView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), text_size),
    );
    text.setEditable(false);
    text.setSelectable(true);
    text.setRichText(false);
    text.setHorizontallyResizable(false);
    text.setVerticallyResizable(true);
    text.setMinSize(NSSize::new(0.0, text_size.height));
    text.setMaxSize(NSSize::new(text_size.width, f64::MAX));
    // SAFETY: NSTextView owns its text container for its entire lifetime.
    if let Some(container) = unsafe { text.textContainer() } {
        container.setContainerSize(NSSize::new(text_size.width, f64::MAX));
        container.setWidthTracksTextView(true);
    }
    text.setString(&NSString::from_str(prompt));
    scroll.setDocumentView(Some(&text));
    // SAFETY: NSView retains its subviews; all run on the main thread.
    view.addSubview(&scroll);

    // SAFETY: NSObject's init signature is correct; the local strong reference
    // keeps the button target and window delegate alive until modal returns.
    let allocated = ConsentDelegate::alloc(mtm).set_ivars(Cell::new(false));
    let delegate: Retained<ConsentDelegate> = unsafe { msg_send![super(allocated), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let decline = NSButton::initWithFrame(
        NSButton::alloc(mtm),
        NSRect::new(NSPoint::new(width - 450.0, 22.0), NSSize::new(270.0, 34.0)),
    );
    let send = NSButton::initWithFrame(
        NSButton::alloc(mtm),
        NSRect::new(NSPoint::new(width - 174.0, 22.0), NSSize::new(158.0, 34.0)),
    );
    for (button, label, action) in [
        (&decline, DECLINE_LABEL, sel!(declineFeedback:)),
        (&send, SEND_LABEL, sel!(sendFeedback:)),
    ] {
        button.setTitle(&NSString::from_str(label));
        button.setBezelStyle(NSBezelStyle::Push);
        // SAFETY: Both selectors are declared above with the correct target
        // signature, and the target is retained until the modal loop ends.
        unsafe {
            button.setTarget(Some(&delegate));
            button.setAction(Some(action));
            view.addSubview(button);
        }
    }
    // Escape and the window close control both decline. Send is never the
    // default Return key action, so merely dismissing the window cannot send.
    decline.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    window.makeKeyAndOrderFront(None);
    let response = app.runModalForWindow(&window);
    window.orderOut(None);
    window.setDelegate(None);
    window.close();
    Ok(response == 1 && delegate.ivars().get())
}
