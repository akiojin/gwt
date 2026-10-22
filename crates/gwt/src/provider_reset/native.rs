//! Human consent from an OS dialog, never from agent-supplied JSON or stdin.

pub(crate) const APPROVE_LABEL: &str = "Redeem free reset";

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn buttons(action: &str) -> rfd::MessageButtons {
    // The first/default and third/Escape buttons both decline. Inverting an
    // OkCancel pair would make Escape look like approval on some backends.
    rfd::MessageButtons::YesNoCancelCustom("Cancel".into(), action.into(), "Close".into())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn approved(action: &str, result: rfd::MessageDialogResult) -> bool {
    matches!(result, rfd::MessageDialogResult::Custom(label) if label == action)
}

pub(crate) fn confirm(prompt: &str) -> Result<bool, String> {
    confirm_with("GWT — Free Codex reset", APPROVE_LABEL, prompt)
}

/// Call on gwtd's main thread, outside an async runtime. Linux deliberately
/// uses GTK directly: rfd's default Linux message backend runs PATH's zenity,
/// which is not an appropriate authority for an agent-requested approval.
#[cfg(not(test))]
pub(crate) fn confirm_with(title: &str, action: &str, prompt: &str) -> Result<bool, String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        Ok(approved(
            action,
            rfd::MessageDialog::new()
                .set_title(title)
                .set_description(prompt)
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(buttons(action))
                .show(),
        ))
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        gtk::init().map_err(|_| "Native confirmation requires an available desktop display")?;
        let dialog = gtk::MessageDialog::builder()
            .title(title)
            .text(if action == APPROVE_LABEL {
                "Redeem one free Codex reset?"
            } else {
                action
            })
            .secondary_text(prompt)
            .message_type(gtk::MessageType::Warning)
            .modal(true)
            .build();
        dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button(action, gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Cancel);
        let response = dialog.run();
        dialog.close();
        Ok(response == gtk::ResponseType::Accept)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (title, prompt);
        Err(if action == APPROVE_LABEL {
            "Native free-reset confirmation is unavailable on this platform"
        } else {
            "Native confirmation is unavailable on this platform"
        }
        .into())
    }
}

#[cfg(test)]
pub(crate) fn confirm_with(_title: &str, _action: &str, _prompt: &str) -> Result<bool, String> {
    panic!("tests must inject consent instead of displaying a native dialog")
}

#[cfg(all(test, any(target_os = "macos", target_os = "windows")))]
mod tests {
    use super::*;

    #[test]
    fn generic_action_requires_its_exact_explicit_button() {
        let action = "Send feedback";
        let accepts = |result| approved(action, result);
        assert!(
            matches!(buttons(action), rfd::MessageButtons::YesNoCancelCustom(first, middle, last)
            if first == "Cancel" && middle == action && last == "Close")
        );
        for label in [
            "Cancel",
            "Close",
            APPROVE_LABEL,
            "send feedback",
            "Send feedback ",
        ] {
            assert!(!accepts(rfd::MessageDialogResult::Custom(label.into())));
        }
        assert!(accepts(rfd::MessageDialogResult::Custom(action.into())));
    }

    #[test]
    fn only_explicit_middle_button_approves() {
        assert!(
            matches!(buttons(APPROVE_LABEL), rfd::MessageButtons::YesNoCancelCustom(first, middle, last)
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
            assert!(!approved(APPROVE_LABEL, result));
        }
        assert!(approved(
            APPROVE_LABEL,
            rfd::MessageDialogResult::Custom(APPROVE_LABEL.into())
        ));
    }
}
