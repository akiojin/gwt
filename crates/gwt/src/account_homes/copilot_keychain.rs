use super::{io, normal_component, COPILOT_KEYCHAIN_SERVICE};
use objc2_core_foundation::{kCFBooleanTrue, CFData, CFDictionary, CFRetained, CFString, CFType};
use objc2_security::{
    errSecDuplicateItem, errSecItemNotFound, kSecAttrAccount, kSecAttrService, kSecClass,
    kSecClassGenericPassword, kSecReturnData, kSecValueData, SecItemAdd, SecItemCopyMatching,
    SecItemDelete, SecItemUpdate,
};
use std::ptr::{self, NonNull};

fn allowed(id: &str) -> bool {
    !cfg!(test) && gwt_config::home::gwt_home_override().is_none() && normal_component(id)
}

pub(crate) fn read(id: &str) -> Option<String> {
    if !allowed(id) {
        return None;
    }
    let id = CFString::from_str(id);
    let service = CFString::from_str(COPILOT_KEYCHAIN_SERVICE);
    let mut result = ptr::null();
    // SAFETY: The query contains valid, retained Core Foundation objects.
    let status = unsafe {
        let query = CFDictionary::<CFString, CFType>::from_slices(
            &[kSecClass, kSecAttrService, kSecAttrAccount, kSecReturnData],
            &[kSecClassGenericPassword, &service, &id, kCFBooleanTrue?],
        );
        SecItemCopyMatching(query.as_opaque(), &mut result)
    };
    if status != 0 {
        return None;
    }
    // SAFETY: A successful CopyMatching returns an owned CF reference.
    let data = unsafe { CFRetained::from_raw(NonNull::new(result.cast_mut())?) }
        .downcast::<CFData>()
        .ok()?;
    String::from_utf8(data.to_vec())
        .ok()
        .filter(|token| !token.is_empty())
}

pub(crate) fn save(id: &str, token: &str, replace: bool) -> io::Result<()> {
    if !allowed(id) {
        return Err(io::Error::other("Copilot Keychain access is unavailable."));
    }
    let id = CFString::from_str(id);
    let service = CFString::from_str(COPILOT_KEYCHAIN_SERVICE);
    let secret = CFData::from_bytes(token.as_bytes());
    // SAFETY: Both dictionaries contain only initialized Core Foundation objects.
    let status = unsafe {
        let values = CFDictionary::<CFString, CFType>::from_slices(
            &[kSecClass, kSecAttrService, kSecAttrAccount, kSecValueData],
            &[kSecClassGenericPassword, &service, &id, &secret],
        );
        let status = SecItemAdd(values.as_opaque(), ptr::null_mut());
        if status == errSecDuplicateItem && replace {
            let query = CFDictionary::<CFString, CFType>::from_slices(
                &[kSecClass, kSecAttrService, kSecAttrAccount],
                &[kSecClassGenericPassword, &service, &id],
            );
            let update =
                CFDictionary::<CFString, CFType>::from_slices(&[kSecValueData], &[&secret]);
            SecItemUpdate(query.as_opaque(), update.as_opaque())
        } else {
            status
        }
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(
            "Could not save the Copilot token in macOS Keychain.",
        ))
    }
}

pub(crate) fn remove(id: &str) {
    if !allowed(id) {
        return;
    }
    let id = CFString::from_str(id);
    let service = CFString::from_str(COPILOT_KEYCHAIN_SERVICE);
    // SAFETY: The query is limited to gwt's service and the validated ID.
    let status = unsafe {
        let query = CFDictionary::<CFString, CFType>::from_slices(
            &[kSecClass, kSecAttrService, kSecAttrAccount],
            &[kSecClassGenericPassword, &service, &id],
        );
        SecItemDelete(query.as_opaque())
    };
    if status != 0 && status != errSecItemNotFound {
        tracing::warn!("Could not remove an unused Copilot Keychain item");
    }
}
