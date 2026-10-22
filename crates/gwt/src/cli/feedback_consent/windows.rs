use std::mem::size_of;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, GetStockObject, MonitorFromPoint, COLOR_WINDOW, DEFAULT_GUI_FONT,
            HBRUSH, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

use super::{bounded_window_size, DECLINE_LABEL, FOOTER_HEIGHT, PADDING, SEND_LABEL, TITLE};

const SEND_ID: usize = 1001;
const DECLINE_ID: usize = 2; // IDCANCEL, including the dialog manager's Escape.

#[derive(Default)]
struct ConsentState {
    approved: bool,
    send_button: HWND,
}

// SAFETY: Only this worker's message loop calls the procedure. The pointer in
// GWLP_USERDATA is a boxed ConsentState kept alive until its HWND is destroyed.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        match message {
            WM_COMMAND => {
                let id = wparam.0 & 0xffff;
                let notification = (wparam.0 >> 16) as u32;
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ConsentState;
                if !state.is_null() && notification == BN_CLICKED {
                    if id == SEND_ID && lparam.0 == (*state).send_button.0 as isize {
                        (*state).approved = true;
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    if id == DECLINE_ID {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                }
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

struct WindowGuard(HWND);

impl Drop for WindowGuard {
    fn drop(&mut self) {
        // A button may already have destroyed it. IsWindow avoids a second
        // destroy and guarantees no callback retains the borrowed state.
        unsafe {
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn confirm(prompt: &str) -> Result<bool, String> {
    // This whole native lifetime belongs to the detached worker's main
    // thread. No source-project window is disabled or used as an owner.
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None).map_err(|e| e.to_string())?.0);
        let class = w!("GWTFeedbackConsentWindow");
        let descriptor = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&descriptor) == 0 {
            return Err("Cannot register the native feedback consent window".into());
        }
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).map_err(|e| e.to_string())?;
        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return Err("Cannot determine the desktop work area".into());
        }
        let work = info.rcWork;
        let size = bounded_window_size(work.right - work.left, work.bottom - work.top)?;
        let mut state = Box::<ConsentState>::default();
        let title = wide(TITLE);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(title.as_ptr()),
            WS_CAPTION | WS_SYSMENU,
            work.left + (work.right - work.left - size.width) / 2,
            work.top + (work.bottom - work.top - size.height) / 2,
            size.width,
            size.height,
            None,
            None,
            Some(instance),
            Some((&mut *state as *mut ConsentState).cast()),
        )
        .map_err(|e| e.to_string())?;
        let guard = WindowGuard(hwnd);
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).map_err(|e| e.to_string())?;
        let width = client.right;
        let height = client.bottom;
        // The fixed buttons are siblings of the scrollable edit control.
        // ES_MULTILINE without ES_AUTOHSCROLL wraps text at the viewport edge.
        let body = wide(&prompt.replace("\r\n", "\n").replace('\n', "\r\n"));
        let text = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
            PADDING,
            PADDING,
            width - 2 * PADDING,
            height - FOOTER_HEIGHT - PADDING,
            Some(hwnd),
            None,
            Some(instance),
            None,
        )
        .map_err(|e| e.to_string())?;
        // WM_SETTEXT is not subject to EDIT's default 32,767-character user
        // input limit. Reject an incomplete native preview before showing any
        // consent action, even if the control ran out of memory.
        SetWindowTextW(text, PCWSTR(body.as_ptr())).map_err(|e| e.to_string())?;
        if GetWindowTextLengthW(text) as usize != body.len() - 1 {
            return Err("Native feedback preview could not display the complete report".into());
        }
        let mut buttons = Vec::new();
        for (label, id, x, button_width) in [
            (DECLINE_LABEL, DECLINE_ID, width - 442, 260),
            (SEND_LABEL, SEND_ID, width - 174, 158),
        ] {
            let label = wide(label);
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(label.as_ptr()),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                x,
                height - 56,
                button_width,
                34,
                Some(hwnd),
                Some(HMENU(id as *mut _)),
                Some(instance),
                None,
            )
            .map_err(|e| e.to_string())?;
            if id == SEND_ID {
                state.send_button = button;
            }
            buttons.push(button);
        }
        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in [text, buttons[0], buttons[1]] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                return Err("Native feedback message loop failed".into());
            }
            if result == 0 {
                break;
            }
            // Capture Escape even while the read-only body has keyboard focus.
            if message.message == WM_KEYDOWN && message.wParam.0 == 0x1b {
                let _ = DestroyWindow(hwnd);
                continue;
            }
            if !IsDialogMessageW(hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        drop(guard);
        Ok(state.approved)
    }
}
