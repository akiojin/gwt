//! A bounded feedback preview. Only its explicit send action grants consent.

#[cfg(not(test))]
const TITLE: &str = "GWT — Send feedback to akiojin/gwt";
#[cfg(not(test))]
const SEND_LABEL: &str = "Send feedback";
#[cfg(not(test))]
const DECLINE_LABEL: &str = "Do not send (save locally)";
const PADDING: i32 = 16;
const FOOTER_HEIGHT: i32 = 76;

#[derive(Debug, PartialEq)]
struct WindowSize {
    width: i32,
    height: i32,
}

fn bounded_window_size(work_width: i32, work_height: i32) -> Result<WindowSize, String> {
    let width = (work_width - 48).min(760);
    let height = (work_height - 48).min(600);
    if width < 480 || height < 300 {
        return Err("Desktop work area is too small to display feedback consent safely".into());
    }
    Ok(WindowSize { width, height })
}

/// AppKit reports switching to the policy a process already has as a failure.
/// A worker launched from an `LSUIElement` bundle (gwtd inside GWT.app) is
/// already Accessory, so only a real switch attempt can fail.
#[cfg_attr(all(not(test), not(target_os = "macos")), allow(dead_code))]
fn accessory_policy_ready(already_accessory: bool, switch: impl FnOnce() -> bool) -> bool {
    already_accessory || switch()
}

fn validate_prompt(prompt: &str) -> Result<(), String> {
    if prompt.contains('\0') {
        return Err("Feedback contains a NUL character that cannot be safely previewed; the report remains saved locally".into());
    }
    Ok(())
}

#[cfg(all(not(test), target_os = "linux"))]
#[path = "feedback_consent/linux.rs"]
mod linux;
#[cfg(all(not(test), target_os = "macos"))]
#[path = "feedback_consent/macos.rs"]
mod macos;
#[cfg(all(not(test), target_os = "windows"))]
#[path = "feedback_consent/windows.rs"]
mod windows;

/// Run only in the detached feedback worker, on its main thread. Waiting here
/// must never borrow the source project's runtime or block its other work.
#[cfg(not(test))]
pub(super) fn confirm(prompt: &str) -> Result<bool, String> {
    validate_prompt(prompt)?;
    #[cfg(target_os = "macos")]
    {
        macos::confirm(prompt)
    }
    #[cfg(target_os = "windows")]
    {
        windows::confirm(prompt)
    }
    #[cfg(target_os = "linux")]
    {
        linux::confirm(prompt)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = prompt;
        Err("Native feedback consent is unavailable on this platform".into())
    }
}

#[cfg(test)]
pub(super) fn confirm(_prompt: &str) -> Result<bool, String> {
    panic!("tests must inject consent instead of displaying a native feedback window")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_feedback_keeps_the_window_and_footer_inside_the_work_area() {
        let prompt = "原文を保持する拒否文\n".repeat(1000);
        assert!(prompt.chars().count() >= 10_000);
        // 1366x768 with OS menu/taskbar space excluded.
        let size = bounded_window_size(1366, 720).expect("desktop has enough room");
        assert!(size.width <= 1366 - 48);
        assert!(size.height <= 720 - 48);
        // Reserve OS chrome, a fixed action footer, and padding below the body.
        assert!(size.height - 48 - FOOTER_HEIGHT - PADDING >= 100);
        assert!(size.width >= 480);
    }

    #[test]
    fn insufficient_work_area_fails_closed_instead_of_hiding_the_actions() {
        assert!(bounded_window_size(400, 200).is_err());
    }

    #[test]
    fn bundled_ui_element_worker_already_accessory_is_ready_without_a_policy_change() {
        // gwtd inside GWT.app (LSUIElement) starts as Accessory; AppKit then
        // reports the no-op switch as false. That must not block consent.
        assert!(accessory_policy_ready(true, || panic!("no switch needed")));
        assert!(accessory_policy_ready(false, || true));
        assert!(!accessory_policy_ready(false, || false));
    }

    #[test]
    fn embedded_nul_cannot_hide_a_suffix_of_the_feedback_preview() {
        assert!(validate_prompt("visible\0hidden submission contents").is_err());
        assert!(validate_prompt("visible\nsubmission contents").is_ok());
    }
}
