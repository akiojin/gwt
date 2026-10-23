//! Native folder selection runs on a dedicated worker, never the tray event loop.

use std::{path::PathBuf, time::Instant};

#[cfg(target_os = "macos")]
#[path = "native_project_picker/macos.rs"]
mod macos_picker;

pub(super) fn pick_project_folder(deadline: Instant) -> Result<Option<PathBuf>, String> {
    if Instant::now() >= deadline {
        return Err("Folder selection timed out".into());
    }
    #[cfg(target_os = "windows")]
    {
        windows_picker::pick(deadline)
    }
    #[cfg(target_os = "macos")]
    {
        macos_picker::pick(deadline)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Ok(rfd::FileDialog::new().pick_folder())
    }
}

#[cfg(target_os = "windows")]
mod windows_picker {
    use super::*;
    use std::{cell::RefCell, ffi::OsString, os::windows::ffi::OsStringExt};
    use windows::{
        core::{w, Interface},
        Win32::{
            Foundation::{ERROR_CANCELLED, HWND, LPARAM, WPARAM},
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
                },
                Ole::IOleWindow,
            },
            UI::{
                Shell::{
                    FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
                    FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
                },
                WindowsAndMessaging::{
                    GetForegroundWindow, KillTimer, PostMessageW, SetForegroundWindow, SetTimer,
                    WM_CLOSE,
                },
            },
        },
    };

    struct ActiveDialog {
        dialog: IFileOpenDialog,
        deadline: Instant,
        foreground_requested: bool,
    }

    thread_local! {
        // Timer callbacks are dispatched by Show's modal loop on this same STA.
        static ACTIVE: RefCell<Option<ActiveDialog>> = const { RefCell::new(None) };
    }

    unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
        let action = ACTIVE.with(|slot| {
            let slot = slot.borrow();
            let active = slot.as_ref()?;
            let foreground = !active.foreground_requested;
            Some((active.dialog.clone(), active.deadline, foreground))
        });
        let Some((dialog, deadline, foreground)) = action else {
            return;
        };
        // Release the RefCell borrow before COM calls, which may pump messages.
        unsafe {
            if let Ok(window) = dialog.cast::<IOleWindow>() {
                if let Ok(hwnd) = window.GetWindow() {
                    if Instant::now() >= deadline {
                        // Post cancellation through the dialog's own message loop.
                        // Calling COM Close from its timer can hide the window
                        // without unwinding Show's nested modal loop.
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    } else if foreground {
                        let _ = SetForegroundWindow(hwnd);
                        ACTIVE.with(|slot| {
                            if let Some(active) = slot.borrow_mut().as_mut() {
                                active.foreground_requested = true;
                            }
                        });
                    }
                }
            }
        }
    }

    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct DialogTimer(usize);
    impl Drop for DialogTimer {
        fn drop(&mut self) {
            unsafe {
                let _ = KillTimer(None, self.0);
            }
            ACTIVE.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub(super) fn pick(deadline: Instant) -> Result<Option<PathBuf>, String> {
        // All COM pointers and callbacks remain on this dedicated STA thread.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|error| error.to_string())?;
            let _apartment = Apartment;
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| error.to_string())?;
            dialog
                .SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                .map_err(|error| error.to_string())?;
            dialog
                .SetTitle(w!("Select project folder"))
                .map_err(|error| error.to_string())?;
            let timer = SetTimer(None, 0, 100, Some(tick));
            if timer == 0 {
                return Err("Cannot start folder selection deadline timer".into());
            }
            let _timer = DialogTimer(timer);
            ACTIVE.with(|slot| {
                *slot.borrow_mut() = Some(ActiveDialog {
                    dialog: dialog.clone(),
                    deadline,
                    foreground_requested: false,
                })
            });
            // The browser is the visible owner in tray mode. The dialog's HWND is
            // activated once it exists; the tray loop remains free throughout.
            let owner = GetForegroundWindow();
            match dialog.Show((!owner.is_invalid()).then_some(owner)) {
                Ok(()) => {
                    let item = dialog.GetResult().map_err(|error| error.to_string())?;
                    let path = item
                        .GetDisplayName(SIGDN_FILESYSPATH)
                        .map_err(|error| error.to_string())?;
                    let result = PathBuf::from(OsString::from_wide(path.as_wide()));
                    CoTaskMemFree(Some(path.0.cast()));
                    Ok(Some(result))
                }
                Err(error) if error.code() == ERROR_CANCELLED.to_hresult() => {
                    if Instant::now() >= deadline {
                        Err("Folder selection timed out".into())
                    } else {
                        Ok(None)
                    }
                }
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_request_does_not_open_a_dialog() {
        assert!(pick_project_folder(Instant::now()).is_err());
    }

    #[cfg(target_os = "windows")]
    mod native {
        use super::*;
        use std::{
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };
        use windows::{
            core::BOOL,
            Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                System::Threading::GetCurrentThreadId,
                UI::WindowsAndMessaging::{
                    EnumThreadWindows, GetForegroundWindow, IsWindow, IsWindowVisible,
                    PostMessageW, WM_CLOSE,
                },
            },
        };

        unsafe extern "system" fn visible_window(hwnd: HWND, state: LPARAM) -> BOOL {
            unsafe {
                if IsWindowVisible(hwnd).as_bool() {
                    *(state.0 as *mut Option<HWND>) = Some(hwnd);
                    return BOOL(0);
                }
            }
            BOOL(1)
        }

        fn run_observed_picker(cancel: bool) -> Result<Option<PathBuf>, String> {
            let picker_thread = unsafe { GetCurrentThreadId() };
            let finished = AtomicBool::new(false);
            let (result, (handle, was_foreground)) = std::thread::scope(|scope| {
                let observer = scope.spawn(|| {
                    let mut handle = 0usize;
                    let mut was_foreground = false;
                    while !finished.load(Ordering::Acquire) {
                        let mut window: Option<HWND> = None;
                        unsafe {
                            let _ = EnumThreadWindows(
                                picker_thread,
                                Some(visible_window),
                                LPARAM((&mut window as *mut Option<HWND>) as isize),
                            );
                            if let Some(hwnd) = window {
                                handle = hwnd.0 as usize;
                                was_foreground |= GetForegroundWindow() == hwnd;
                                if cancel && was_foreground {
                                    let _ =
                                        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    (handle, was_foreground)
                });
                let result = pick_project_folder(Instant::now() + Duration::from_secs(3));
                finished.store(true, Ordering::Release);
                (result, observer.join().expect("native window observer"))
            });
            assert_ne!(handle, 0, "native picker must become visible");
            assert!(
                was_foreground,
                "native picker must be the foreground window"
            );
            assert!(
                !unsafe { IsWindow(Some(HWND(handle as *mut _))) }.as_bool(),
                "native picker must be destroyed on completion"
            );
            result
        }

        #[test]
        #[ignore = "opens a real native Windows dialog; run during native UI verification"]
        fn native_timeout_closes_dialog() {
            assert_eq!(
                run_observed_picker(false),
                Err("Folder selection timed out".into())
            );
        }

        #[test]
        #[ignore = "opens a real native Windows dialog; run during native UI verification"]
        fn native_cancel_closes_dialog() {
            assert_eq!(run_observed_picker(true), Ok(None));
        }
    }
}
