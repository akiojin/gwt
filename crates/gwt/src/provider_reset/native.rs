//! Human consent from an OS dialog, never from agent-supplied JSON or stdin.

#[cfg(any(
    target_os = "macos",
    target_os = "windows",
    all(target_os = "linux", not(test))
))]
pub(crate) const APPROVE_LABEL: &str = "Redeem free reset";

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn buttons() -> rfd::MessageButtons {
    // The first/default and third/Escape buttons both decline. Inverting an
    // OkCancel pair would make Escape look like approval on some backends.
    rfd::MessageButtons::YesNoCancelCustom("Cancel".into(), APPROVE_LABEL.into(), "Close".into())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn approved(result: rfd::MessageDialogResult) -> bool {
    matches!(result, rfd::MessageDialogResult::Custom(label) if label == APPROVE_LABEL)
}

/// Call on gwtd's main thread, outside an async runtime. Linux deliberately
/// uses GTK directly: rfd's default Linux message backend runs PATH's zenity,
/// which is not an appropriate authority for an agent-requested approval.
#[cfg(not(test))]
pub(crate) fn confirm(prompt: &str) -> Result<bool, String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        Ok(approved(
            rfd::MessageDialog::new()
                .set_title("GWT — Free Codex reset")
                .set_description(prompt)
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(buttons())
                .show(),
        ))
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        gtk::init().map_err(|_| "Native confirmation requires an available desktop display")?;
        let dialog = gtk::MessageDialog::builder()
            .title("GWT — Free Codex reset")
            .text("Redeem one free Codex reset?")
            .secondary_text(prompt)
            .message_type(gtk::MessageType::Warning)
            .modal(true)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button(APPROVE_LABEL, gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Cancel);
        let response = dialog.run();
        dialog.close();
        Ok(response == gtk::ResponseType::Accept)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = prompt;
        Err("Native free-reset confirmation is unavailable on this platform".into())
    }
}

#[cfg(test)]
pub(crate) fn confirm(_prompt: &str) -> Result<bool, String> {
    panic!("tests must inject consent instead of displaying a native dialog")
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_middle_button_approves() {
        assert!(
            matches!(buttons(), rfd::MessageButtons::YesNoCancelCustom(first, middle, last)
            if first == "Cancel" && middle == APPROVE_LABEL && last == "Close")
        );
        for result in [
            rfd::MessageDialogResult::Cancel,
            rfd::MessageDialogResult::Ok,
            rfd::MessageDialogResult::Yes,
            rfd::MessageDialogResult::No,
            rfd::MessageDialogResult::Custom("Cancel".into()),
            rfd::MessageDialogResult::Custom("Close".into()),
        ] {
            assert!(!approved(result));
        }
        assert!(approved(rfd::MessageDialogResult::Custom(
            APPROVE_LABEL.into()
        )));
    }
}
