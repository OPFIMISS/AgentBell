#[cfg(target_os = "windows")]
mod platform {
    use std::{
        mem::zeroed,
        ptr::{null, null_mut},
        sync::{
            OnceLock,
            atomic::{AtomicU8, AtomicU32, Ordering},
        },
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DispatchMessageW, GetCursorPos, GetMessageW, IDI_APPLICATION, LoadIconW, MF_STRING,
                MSG, PostQuitMessage, RegisterClassW, SetForegroundWindow, TPM_NONOTIFY,
                TPM_RETURNCMD, TrackPopupMenu, TranslateMessage, WM_APP, WM_LBUTTONDBLCLK,
                WM_RBUTTONUP, WNDCLASSW,
            },
        },
    };

    static WEB_URL: OnceLock<String> = OnceLock::new();
    static STATE: AtomicU8 = AtomicU8::new(0);
    static ERROR: AtomicU32 = AtomicU32::new(0);
    const WM_TRAY: u32 = WM_APP + 83;
    const OPEN_ID: usize = 1;
    const EXIT_ID: usize = 2;

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_TRAY && wparam == 1 {
            let action = lparam as u32;
            if action == WM_LBUTTONDBLCLK {
                if let Some(url) = WEB_URL.get() {
                    let _ = open::that(url);
                }
                return 0;
            }
            if action == WM_RBUTTONUP {
                let menu = unsafe { CreatePopupMenu() };
                if !menu.is_null() {
                    let open_text = wide("打开 AgentBell");
                    let exit_text = wide("退出 AgentBell");
                    unsafe {
                        AppendMenuW(menu, MF_STRING, OPEN_ID, open_text.as_ptr());
                        AppendMenuW(menu, MF_STRING, EXIT_ID, exit_text.as_ptr());
                        let mut point: POINT = zeroed();
                        GetCursorPos(&mut point);
                        SetForegroundWindow(hwnd);
                        let command = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_NONOTIFY,
                            point.x,
                            point.y,
                            0,
                            hwnd,
                            null(),
                        );
                        DestroyMenu(menu);
                        if command == OPEN_ID as i32 {
                            if let Some(url) = WEB_URL.get() {
                                let _ = open::that(url);
                            }
                        } else if command == EXIT_ID as i32 {
                            let mut icon: NOTIFYICONDATAW = zeroed();
                            icon.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                            icon.hWnd = hwnd;
                            icon.uID = 1;
                            Shell_NotifyIconW(NIM_DELETE, &icon);
                            PostQuitMessage(0);
                            std::process::exit(0);
                        }
                    }
                }
                return 0;
            }
        }
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    pub fn start(url: String) {
        let _ = WEB_URL.set(url);
        std::thread::spawn(|| unsafe {
            STATE.store(1, Ordering::Relaxed);
            let class_name = wide("AgentBellTrayWindow");
            let instance = GetModuleHandleW(null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..zeroed()
            };
            if RegisterClassW(&class) == 0 {
                ERROR.store(
                    windows_sys::Win32::Foundation::GetLastError(),
                    Ordering::Relaxed,
                );
                return;
            }
            STATE.store(2, Ordering::Relaxed);
            let title = wide("AgentBell");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if hwnd.is_null() {
                ERROR.store(
                    windows_sys::Win32::Foundation::GetLastError(),
                    Ordering::Relaxed,
                );
                return;
            }
            STATE.store(3, Ordering::Relaxed);
            let mut icon: NOTIFYICONDATAW = zeroed();
            icon.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            icon.hWnd = hwnd;
            icon.uID = 1;
            icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            icon.uCallbackMessage = WM_TRAY;
            icon.hIcon = LoadIconW(instance, 1usize as *const u16);
            if icon.hIcon.is_null() {
                icon.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
            }
            let tip = wide("AgentBell - Agent 完成通知");
            let tip_len = tip.len().min(icon.szTip.len());
            icon.szTip[..tip_len].copy_from_slice(&tip[..tip_len]);
            if Shell_NotifyIconW(NIM_ADD, &icon) == 0 {
                ERROR.store(
                    windows_sys::Win32::Foundation::GetLastError(),
                    Ordering::Relaxed,
                );
                return;
            }
            STATE.store(4, Ordering::Relaxed);
            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            Shell_NotifyIconW(NIM_DELETE, &icon);
        });
    }

    pub fn status() -> String {
        format!(
            "state={} error={}",
            STATE.load(Ordering::Relaxed),
            ERROR.load(Ordering::Relaxed)
        )
    }
}

#[cfg(target_os = "windows")]
pub use platform::start;
#[cfg(target_os = "windows")]
pub use platform::status;

#[cfg(not(target_os = "windows"))]
pub fn start(_url: String) {}
#[cfg(not(target_os = "windows"))]
pub fn status() -> String {
    "unsupported".into()
}
