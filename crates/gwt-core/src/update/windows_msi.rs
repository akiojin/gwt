//! Read-only admission for unattended Windows MSI updates.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::ApplicationInstallationAndServicing as msi;

const GWT_UPGRADE_CODE: &str = "{55555555-5555-5555-5555-555555555555}";

pub(super) fn automatic_apply_refusal(installer: &Path, target_exe: &Path) -> Result<(), String> {
    let legacy_roots: Vec<_> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    automatic_apply_refusal_with(installer, target_exe, &legacy_roots, registered_component)
}

fn automatic_apply_refusal_with(
    installer: &Path,
    target_exe: &Path,
    legacy_roots: &[PathBuf],
    registered_component: impl Fn(&str, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let component = read_package_component(installer)?;
    registered_component(&component, target_exe)?;
    if legacy_roots.is_empty() {
        return Err(
            "Program Files locations could not be verified; apply the update manually.".into(),
        );
    }
    for root in legacy_roots {
        let exists = root.join("GWT").join("gwt.exe").try_exists().map_err(|_| {
            "Legacy Program Files installation could not be checked; apply the update manually."
                .to_string()
        })?;
        if exists {
            return Err(
                "A legacy Program Files installation requires explicit manual migration.".into(),
            );
        }
    }
    if !target_exe.parent().is_some_and(super::dir_writable) {
        return Err(
            "The current GWT installation directory is not writable; apply the update manually."
                .into(),
        );
    }
    Ok(())
}

struct Handle(msi::MSIHANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the handle returned by a successful MSI call.
        unsafe { msi::MsiCloseHandle(self.0) };
    }
}

fn checked(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "Windows MSI {operation} failed (error {code}); apply the update manually."
        ))
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn query_text(database: msi::MSIHANDLE, query: &str) -> Result<Option<String>, String> {
    let query = wide(query);
    let mut view = msi::MSIHANDLE::default();
    // SAFETY: the database is live and the query is NUL terminated; MSI owns no input buffers.
    unsafe {
        checked(
            msi::MsiDatabaseOpenViewW(database, PCWSTR(query.as_ptr()), &mut view),
            "metadata query",
        )?;
        let view = Handle(view);
        checked(
            msi::MsiViewExecute(view.0, msi::MSIHANDLE::default()),
            "metadata query execution",
        )?;
        let mut record = msi::MSIHANDLE::default();
        let code = msi::MsiViewFetch(view.0, &mut record);
        if code == 259 {
            // ERROR_NO_MORE_ITEMS: an absent optional property is empty.
            return Ok(None);
        }
        checked(code, "metadata record")?;
        let record = Handle(record);
        let mut buffer = vec![0u16; 1024];
        let mut length = buffer.len() as u32;
        checked(
            msi::MsiRecordGetStringW(
                record.0,
                1,
                Some(PWSTR(buffer.as_mut_ptr())),
                Some(&mut length),
            ),
            "metadata value",
        )?;
        String::from_utf16(&buffer[..length as usize])
            .map(Some)
            .map_err(|_| {
                "Windows MSI metadata is not valid Unicode; apply the update manually.".into()
            })
    }
}

fn read_package_component(installer: &Path) -> Result<String, String> {
    let path: Vec<u16> = installer.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut database = msi::MSIHANDLE::default();
    // SAFETY: the path is NUL terminated and the package is opened read-only.
    unsafe {
        checked(
            msi::MsiOpenDatabaseW(
                PCWSTR(path.as_ptr()),
                msi::MSIDBOPEN_READONLY,
                &mut database,
            ),
            "package open",
        )?;
    }
    let database = Handle(database);
    let property = |name: &str| {
        query_text(
            database.0,
            &format!("SELECT `Value` FROM `Property` WHERE `Property` = '{name}'"),
        )
    };
    if property("ProductName")?.as_deref() != Some("GWT")
        || !property("UpgradeCode")?.is_some_and(|code| code.eq_ignore_ascii_case(GWT_UPGRADE_CODE))
    {
        return Err(
            "Windows MSI is not a recognized GWT package; apply the update manually.".into(),
        );
    }
    if property("ALLUSERS")?.is_some_and(|scope| !scope.is_empty()) {
        return Err("Windows MSI is not exclusively per-user; apply the update manually.".into());
    }
    let mut summary = msi::MSIHANDLE::default();
    let mut value_type = 0;
    let mut word_count = 0;
    // SAFETY: the database is live; the summary is opened without update capacity.
    unsafe {
        checked(
            msi::MsiGetSummaryInformationW(database.0, PCWSTR::null(), 0, &mut summary),
            "summary open",
        )?;
        let summary = Handle(summary);
        checked(
            msi::MsiSummaryInfoGetPropertyW(
                summary.0,
                msi::PID_WORDCOUNT,
                &mut value_type,
                &mut word_count,
                None,
                None,
                None,
            ),
            "privilege metadata",
        )?;
    }
    // PID_WORDCOUNT bit 3 explicitly declares that elevation is unnecessary.
    if value_type != 3 || word_count & 8 == 0 {
        return Err(
            "Windows MSI may require administrator privileges; apply the update manually.".into(),
        );
    }
    query_text(
        database.0,
        "SELECT `ComponentId` FROM `Component` WHERE `Component` = 'GwtExecutableComponent'",
    )?
    .filter(|component| uuid::Uuid::parse_str(component).is_ok())
    .ok_or_else(|| {
        "Windows MSI executable component could not be verified; apply the update manually.".into()
    })
}

fn registered_component(component: &str, target_exe: &Path) -> Result<(), String> {
    let target = std::fs::canonicalize(target_exe).map_err(|_| {
        "Current GWT executable path could not be verified; apply the update manually.".to_string()
    })?;
    let component = wide(component);
    let mut index = 0;
    loop {
        let mut product = [0u16; 39];
        let mut context = msi::MSIINSTALLCONTEXT_NONE;
        // SAFETY: buffers have documented sizes. NULL SID limits user contexts to the current user.
        let code = unsafe {
            msi::MsiEnumClientsExW(
                PCWSTR(component.as_ptr()),
                PCWSTR::null(),
                msi::MSIINSTALLCONTEXT_ALL,
                index,
                Some(&mut product),
                Some(&mut context),
                None,
                None,
            )
        };
        if code == 259 {
            break;
        }
        checked(code, "installed scope enumeration")?;
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        // SAFETY: product/component are NUL terminated; the output buffer matches its declared size.
        let state = unsafe {
            msi::MsiGetComponentPathExW(
                PCWSTR(product.as_ptr()),
                PCWSTR(component.as_ptr()),
                PCWSTR::null(),
                Some(context),
                Some(PWSTR(buffer.as_mut_ptr())),
                Some(&mut length),
            )
        };
        if state == msi::INSTALLSTATE_LOCAL {
            if let Ok(path) = String::from_utf16(&buffer[..length as usize]) {
                if let Ok(path) = std::fs::canonicalize(path) {
                    if let Some(result) = registration_result(context, &path, &target) {
                        return result;
                    }
                }
            }
        }
        index += 1;
    }
    Err("Current GWT installation scope could not be verified as per-user; apply the update manually.".into())
}

fn registration_result(
    context: msi::MSIINSTALLCONTEXT,
    registered_exe: &Path,
    target_exe: &Path,
) -> Option<Result<(), String>> {
    super::windows_paths_equal(registered_exe, target_exe).then(|| {
        if context == msi::MSIINSTALLCONTEXT_USERUNMANAGED {
            Ok(())
        } else {
            Err(
                "Current GWT installation is per-machine or managed; apply the update manually."
                    .into(),
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use windows::Win32::System::ApplicationInstallationAndServicing::*;

    const COMPONENT: &str = "{11111111-2222-3333-4444-555555555555}";

    #[test]
    fn issue_5343_registered_scope_fixture_requires_the_current_per_user_executable() {
        let target = Path::new("C:/fixture/user/gwt.exe");
        assert_eq!(
            registration_result(MSIINSTALLCONTEXT_USERUNMANAGED, target, target),
            Some(Ok(()))
        );
        for context in [MSIINSTALLCONTEXT_MACHINE, MSIINSTALLCONTEXT_USERMANAGED] {
            assert!(registration_result(context, target, target)
                .unwrap()
                .unwrap_err()
                .contains("per-machine or managed"));
        }
        assert!(
            registration_result(
                MSIINSTALLCONTEXT_MACHINE,
                Path::new("C:/fixture/old-machine/gwt.exe"),
                target
            )
            .is_none(),
            "a leftover machine registration does not describe the running per-user executable"
        );
    }

    fn fixture_msi(dir: &Path, name: &str, all_users: Option<&str>, word_count: i32) -> PathBuf {
        let path = dir.join(name);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut db = MSIHANDLE::default();
        unsafe {
            assert_eq!(
                MsiOpenDatabaseW(PCWSTR(wide.as_ptr()), MSIDBOPEN_CREATE, &mut db),
                0
            );
            for sql in [
                "CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) LOCALIZABLE PRIMARY KEY `Property`)".to_string(),
                "CREATE TABLE `Component` (`Component` CHAR(72) NOT NULL, `ComponentId` CHAR(38) PRIMARY KEY `Component`)".to_string(),
                "INSERT INTO `Property` (`Property`, `Value`) VALUES ('ProductName', 'GWT')".to_string(),
                "INSERT INTO `Property` (`Property`, `Value`) VALUES ('UpgradeCode', '{55555555-5555-5555-5555-555555555555}')".to_string(),
                format!("INSERT INTO `Component` (`Component`, `ComponentId`) VALUES ('GwtExecutableComponent', '{COMPONENT}')"),
            ] {
                execute(db, &sql);
            }
            if let Some(value) = all_users {
                execute(db, &format!("INSERT INTO `Property` (`Property`, `Value`) VALUES ('ALLUSERS', '{value}')"));
            }
            let mut summary = MSIHANDLE::default();
            assert_eq!(
                MsiGetSummaryInformationW(db, PCWSTR::null(), 1, &mut summary),
                0
            );
            assert_eq!(
                MsiSummaryInfoSetPropertyW(
                    summary,
                    PID_WORDCOUNT,
                    3,
                    word_count,
                    std::ptr::null_mut(),
                    PCWSTR::null()
                ),
                0
            );
            assert_eq!(MsiSummaryInfoPersist(summary), 0);
            assert_eq!(MsiCloseHandle(summary), 0);
            assert_eq!(MsiDatabaseCommit(db), 0);
            assert_eq!(MsiCloseHandle(db), 0);
        }
        path
    }

    unsafe fn execute(db: MSIHANDLE, sql: &str) {
        let wide: Vec<u16> = sql.encode_utf16().chain(Some(0)).collect();
        let mut view = MSIHANDLE::default();
        assert_eq!(
            MsiDatabaseOpenViewW(db, PCWSTR(wide.as_ptr()), &mut view),
            0,
            "{sql}"
        );
        assert_eq!(MsiViewExecute(view, MSIHANDLE::default()), 0, "{sql}");
        assert_eq!(MsiCloseHandle(view), 0);
    }

    #[test]
    fn issue_5343_native_msi_metadata_requires_per_user_without_uac() {
        let temp = tempfile::tempdir().unwrap();
        let per_user = fixture_msi(temp.path(), "user.msi", None, 10);
        assert_eq!(read_package_component(&per_user).unwrap(), COMPONENT);

        let per_machine = fixture_msi(temp.path(), "machine.msi", Some("1"), 10);
        assert!(read_package_component(&per_machine)
            .unwrap_err()
            .contains("per-user"));
        let privileged = fixture_msi(temp.path(), "privileged.msi", None, 2);
        assert!(read_package_component(&privileged)
            .unwrap_err()
            .contains("administrator"));
        let invalid = temp.path().join("invalid.msi");
        fs::write(&invalid, b"not an MSI database").unwrap();
        assert!(read_package_component(&invalid)
            .unwrap_err()
            .contains("MSI"));
    }

    #[test]
    fn issue_5343_msi_admission_requires_registered_writable_per_user_target() {
        let temp = tempfile::tempdir().unwrap();
        let msi = fixture_msi(temp.path(), "user.msi", None, 10);
        let install = temp.path().join("user-install");
        fs::create_dir(&install).unwrap();
        let target = install.join("gwt.exe");
        fs::write(&target, b"fixture").unwrap();
        let legacy_roots = [temp.path().join("Program Files")];

        automatic_apply_refusal_with(&msi, &target, &legacy_roots, |component, actual| {
            assert_eq!(component, COMPONENT);
            assert_eq!(actual, target);
            Ok(())
        })
        .unwrap();

        for refusal in [
            "Current GWT installation is per-machine",
            "Current GWT installation scope could not be verified",
        ] {
            let error = automatic_apply_refusal_with(&msi, &target, &legacy_roots, |_, _| {
                Err(refusal.into())
            })
            .unwrap_err();
            assert_eq!(error, refusal);
        }

        let missing = temp.path().join("unwritable-missing").join("gwt.exe");
        let error =
            automatic_apply_refusal_with(&msi, &missing, &legacy_roots, |_, _| Ok(())).unwrap_err();
        assert!(error.contains("writable"), "{error}");
    }

    #[test]
    fn issue_5343_msi_admission_preserves_legacy_migration_guard() {
        let temp = tempfile::tempdir().unwrap();
        let msi = fixture_msi(temp.path(), "user.msi", None, 10);
        let target = temp.path().join("gwt.exe");
        fs::write(&target, b"fixture").unwrap();
        let legacy_root = temp.path().join("Program Files");
        fs::create_dir_all(legacy_root.join("GWT")).unwrap();
        let legacy_exe = legacy_root.join("GWT").join("gwt.exe");
        fs::write(&legacy_exe, b"legacy fixture").unwrap();

        let error =
            automatic_apply_refusal_with(&msi, &target, &[legacy_root], |_, _| Ok(())).unwrap_err();
        assert!(error.contains("migration"), "{error}");
        assert_eq!(fs::read(&legacy_exe).unwrap(), b"legacy fixture");
    }
}
