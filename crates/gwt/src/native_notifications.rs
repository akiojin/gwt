//! Native desktop delivery for notifications whose browser capability is still valid.

use std::{future::Future, sync::Arc};

use crate::native_notification_permission::{
    query_native_notification_permission, NativeNotificationPermission,
};

type DeliveryGuard = Arc<dyn Fn() -> bool + Send + Sync>;

/// Deliver only with a current native grant and a still-valid browser capability.
///
/// This never requests permission. Suppressed delivery returns `Ok(())`;
/// native failures return an error without falling back to another channel.
/// The future waits for activation or dismissal, so callers should spawn it.
pub async fn deliver_native_notification(
    id: String,
    title: String,
    body: String,
    on_activate: Arc<dyn Fn() + Send + Sync>,
    can_deliver: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<(), String> {
    let send_guard = can_deliver.clone();
    deliver_with_permission(query_native_notification_permission, can_deliver, || {
        send_native_notification(id, title, body, on_activate, send_guard)
    })
    .await
}

async fn deliver_with_permission<Q, S, F>(
    query: Q,
    can_deliver: DeliveryGuard,
    send: S,
) -> Result<(), String>
where
    Q: FnOnce() -> NativeNotificationPermission + Send + 'static,
    S: FnOnce() -> F,
    F: Future<Output = Result<(), String>>,
{
    if !can_deliver() {
        return Ok(());
    }
    let permission = tokio::task::spawn_blocking(query)
        .await
        .map_err(|error| error.to_string())?;
    if !permission.can_notify() {
        tracing::debug!(?permission, "native notification authorization unavailable");
        return Ok(());
    }
    if !can_deliver() {
        return Ok(());
    }
    send().await
}

#[cfg(target_os = "macos")]
async fn send_native_notification(
    id: String,
    title: String,
    body: String,
    on_activate: Arc<dyn Fn() + Send + Sync>,
    can_deliver: DeliveryGuard,
) -> Result<(), String> {
    let notification = mac_usernotifications::Notification::new()
        .id(&id)
        .title(title)
        .message(body);
    if !can_deliver() {
        return Ok(());
    }
    let handle = notification
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let response = handle.response().await.map_err(|error| error.to_string())?;
    if response.is_default_action() {
        on_activate();
    }
    Ok(())
}

#[cfg(windows)]
async fn send_native_notification(
    _id: String,
    title: String,
    body: String,
    on_activate: Arc<dyn Fn() + Send + Sync>,
    can_deliver: DeliveryGuard,
) -> Result<(), String> {
    use std::sync::Mutex;
    use windows::{
        core::{IInspectable, HSTRING},
        Data::Xml::Dom::XmlDocument,
        Foundation::TypedEventHandler,
        UI::Notifications::{
            ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification,
            ToastNotificationManager,
        },
    };

    let (response_tx, response_rx) = tokio::sync::oneshot::channel::<Result<bool, String>>();
    let completion = Arc::new(Mutex::new(Some(response_tx)));
    let complete = move |response| {
        if let Some(sender) = completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = sender.send(response);
        }
    };
    let notification = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let document = XmlDocument::new().map_err(|error| error.to_string())?;
        document
            .LoadXml(&HSTRING::from(notification_xml(&title, &body)))
            .map_err(|error| error.to_string())?;
        let notification = ToastNotification::CreateToastNotification(&document)
            .map_err(|error| error.to_string())?;

        let activated = complete.clone();
        notification
            .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_, _| {
                    activated(Ok(true));
                    Ok(())
                },
            ))
            .map_err(|error| error.to_string())?;
        let dismissed = complete.clone();
        notification
            .Dismissed(&TypedEventHandler::<
                ToastNotification,
                ToastDismissedEventArgs,
            >::new(move |_, args| {
                // A faded banner can still be activated from Notification Center.
                let timed_out = args.as_ref().is_some_and(|args| {
                    args.Reason()
                        .is_ok_and(|reason| reason == ToastDismissalReason::TimedOut)
                });
                if !timed_out {
                    dismissed(Ok(false));
                }
                Ok(())
            }))
            .map_err(|error| error.to_string())?;
        notification
            .Failed(
                &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, _| {
                    complete(Err(
                        "Windows failed to display the native notification".into()
                    ));
                    Ok(())
                }),
            )
            .map_err(|error| error.to_string())?;
        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(WINDOWS_APP_ID))
                .map_err(|error| error.to_string())?;
        if !can_deliver() {
            return Ok(None);
        }
        notifier
            .Show(&notification)
            .map_err(|error| error.to_string())?;
        Ok(Some(notification))
    })
    .await
    .map_err(|error| error.to_string())??;
    let Some(notification) = notification else {
        return Ok(());
    };
    let response = response_rx.await.map_err(|error| error.to_string())?;
    // Retain the native object and its event subscriptions until a terminal response.
    drop(notification);
    if response? {
        on_activate();
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
async fn send_native_notification(
    _id: String,
    _title: String,
    _body: String,
    _on_activate: Arc<dyn Fn() + Send + Sync>,
    _can_deliver: DeliveryGuard,
) -> Result<(), String> {
    // Linux has no supported native authorization query, so delivery stays closed.
    Ok(())
}

#[cfg(any(windows, test))]
const WINDOWS_APP_ID: &str = "io.github.akiojin.gwt";

#[cfg(any(windows, test))]
fn notification_xml(title: &str, body: &str) -> String {
    fn escape(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&apos;"),
                '\t'
                | '\n'
                | '\r'
                | '\u{20}'..='\u{d7ff}'
                | '\u{e000}'..='\u{fffd}'
                | '\u{10000}'..='\u{10ffff}' => escaped.push(character),
                _ => {}
            }
        }
        escaped
    }
    format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>", escape(title), escape(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_notification_permission::MacAuthorizationStatus;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn granted() -> NativeNotificationPermission {
        NativeNotificationPermission::MacOs(MacAuthorizationStatus::Authorized)
    }

    #[tokio::test]
    async fn explicit_permission_dispatches_once() {
        let sent = AtomicUsize::new(0);
        deliver_with_permission(granted, Arc::new(|| true), || async {
            sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_or_denied_permission_never_dispatches() {
        for permission in [
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::NotDetermined),
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Denied),
            NativeNotificationPermission::UnsupportedAuthorization,
            NativeNotificationPermission::QueryFailed("no identity".into()),
        ] {
            deliver_with_permission(
                move || permission,
                Arc::new(|| true),
                || async { panic!("an ungranted permission reached native delivery") },
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn capability_invalidated_while_delivery_is_pending_never_dispatches() {
        let valid = Arc::new(AtomicBool::new(true));
        let delivery_valid = valid.clone();
        let (query_started, query_observed) = tokio::sync::oneshot::channel();
        let (resume_query, query_resume) = std::sync::mpsc::channel();
        let delivery = tokio::spawn(deliver_with_permission(
            move || {
                query_started.send(()).unwrap();
                query_resume.recv().unwrap();
                granted()
            },
            Arc::new(move || delivery_valid.load(Ordering::SeqCst)),
            || async { panic!("an invalidated capability reached native delivery") },
        ));
        query_observed.await.unwrap();
        valid.store(false, Ordering::SeqCst);
        resume_query.send(()).unwrap();
        delivery.await.unwrap().unwrap();
    }

    #[test]
    fn windows_xml_escapes_reserved_characters_and_removes_invalid_xml_characters() {
        assert_eq!(
            notification_xml("<&>\"'\0", "line\n\t\r\u{1f}🙂"),
            "<toast><visual><binding template=\"ToastGeneric\"><text>&lt;&amp;&gt;&quot;&apos;</text><text>line\n\t\r🙂</text></binding></visual></toast>"
        );
    }

    #[test]
    fn windows_installer_registers_the_notification_app_identity_on_its_shortcut() {
        use quick_xml::{events::Event, Reader};

        let mut reader = Reader::from_str(include_str!("../../../wix/main.wxs"));
        let mut gwt_shortcut = false;
        let mut registered = false;
        loop {
            match reader.read_event().unwrap() {
                Event::Start(element) if element.name().as_ref() == "Shortcut" => {
                    gwt_shortcut = element.attributes().flatten().any(|attribute| {
                        attribute.key.as_ref() == "Id"
                            && attribute.value.as_ref() == "GwtStartMenuShortcut"
                    });
                }
                Event::Empty(element)
                    if gwt_shortcut && element.name().as_ref() == "ShortcutProperty" =>
                {
                    let attributes = element.attributes().flatten().collect::<Vec<_>>();
                    registered |= attributes.iter().any(|attribute| {
                        attribute.key.as_ref() == "Key"
                            && attribute.value.as_ref() == "System.AppUserModel.ID"
                    }) && attributes.iter().any(|attribute| {
                        attribute.key.as_ref() == "Value"
                            && attribute.value.as_ref() == WINDOWS_APP_ID
                    });
                }
                Event::End(element) if element.name().as_ref() == "Shortcut" => {
                    gwt_shortcut = false;
                }
                Event::Eof => break,
                _ => {}
            }
        }
        assert!(registered, "GWT's shortcut must own its notification AUMID");
    }
}
