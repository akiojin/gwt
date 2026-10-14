//! Query-only native notification authorization. Never requests permission or sends a notification.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacAuthorizationStatus {
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
    Ephemeral,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsNotificationSetting {
    Enabled,
    DisabledForApplication,
    DisabledForUser,
    DisabledByGroupPolicy,
    DisabledByManifest,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeNotificationPermission {
    MacOs(MacAuthorizationStatus),
    Windows(WindowsNotificationSetting),
    /// The platform has no supported authorization query. In particular, Linux
    /// GetCapabilities describes delivery features, not a user's permission.
    UnsupportedAuthorization,
    QueryFailed(String),
}

impl NativeNotificationPermission {
    pub fn can_notify(&self) -> bool {
        matches!(
            self,
            Self::MacOs(MacAuthorizationStatus::Authorized)
                | Self::Windows(WindowsNotificationSetting::Enabled)
        )
    }
}

/// Queries current settings without prompting. On macOS this blocks while the
/// system returns its settings; callers should keep it off the UI thread.
#[cfg(target_os = "macos")]
pub fn query_native_notification_permission() -> NativeNotificationPermission {
    use mac_usernotifications::AuthorizationStatus;

    // UNUserNotificationCenter can crash without a bundle identity. The library
    // checks it before touching the notification center, including in dev builds.
    match mac_usernotifications::blocking::get_notification_settings() {
        Ok(settings) => NativeNotificationPermission::MacOs(match settings.authorization_status {
            AuthorizationStatus::NotDetermined => MacAuthorizationStatus::NotDetermined,
            AuthorizationStatus::Denied => MacAuthorizationStatus::Denied,
            AuthorizationStatus::Authorized => MacAuthorizationStatus::Authorized,
            AuthorizationStatus::Provisional => MacAuthorizationStatus::Provisional,
            AuthorizationStatus::Ephemeral => MacAuthorizationStatus::Ephemeral,
            AuthorizationStatus::Unknown => MacAuthorizationStatus::Unknown,
        }),
        Err(error) => NativeNotificationPermission::QueryFailed(error.to_string()),
    }
}

#[cfg(target_os = "windows")]
pub fn query_native_notification_permission() -> NativeNotificationPermission {
    use windows::{
        core::HSTRING,
        UI::Notifications::{NotificationSetting, ToastNotificationManager},
    };

    // Query this application's identity, never a surrogate system application.
    let result = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
        "io.github.akiojin.gwt",
    ))
    .and_then(|notifier| notifier.Setting());
    match result {
        Ok(setting) => NativeNotificationPermission::Windows(match setting {
            NotificationSetting::Enabled => WindowsNotificationSetting::Enabled,
            NotificationSetting::DisabledForApplication => {
                WindowsNotificationSetting::DisabledForApplication
            }
            NotificationSetting::DisabledForUser => WindowsNotificationSetting::DisabledForUser,
            NotificationSetting::DisabledByGroupPolicy => {
                WindowsNotificationSetting::DisabledByGroupPolicy
            }
            NotificationSetting::DisabledByManifest => {
                WindowsNotificationSetting::DisabledByManifest
            }
            _ => WindowsNotificationSetting::Unknown,
        }),
        Err(error) => NativeNotificationPermission::QueryFailed(error.to_string()),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn query_native_notification_permission() -> NativeNotificationPermission {
    NativeNotificationPermission::UnsupportedAuthorization
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_native_grants_allow_notifications() {
        assert!(
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Authorized).can_notify()
        );
        assert!(
            NativeNotificationPermission::Windows(WindowsNotificationSetting::Enabled).can_notify()
        );
        for permission in [
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::NotDetermined),
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Denied),
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Provisional),
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Ephemeral),
            NativeNotificationPermission::MacOs(MacAuthorizationStatus::Unknown),
            NativeNotificationPermission::Windows(
                WindowsNotificationSetting::DisabledForApplication,
            ),
            NativeNotificationPermission::Windows(WindowsNotificationSetting::DisabledForUser),
            NativeNotificationPermission::Windows(
                WindowsNotificationSetting::DisabledByGroupPolicy,
            ),
            NativeNotificationPermission::Windows(WindowsNotificationSetting::DisabledByManifest),
            NativeNotificationPermission::Windows(WindowsNotificationSetting::Unknown),
            NativeNotificationPermission::UnsupportedAuthorization,
            NativeNotificationPermission::QueryFailed("no application identity".into()),
        ] {
            assert!(!permission.can_notify(), "{permission:?}");
        }
    }
}
