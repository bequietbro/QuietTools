use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM, ERROR_ALREADY_EXISTS};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_SHIFT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, MessageBoxW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, HWND_MESSAGE, IDC_ARROW,
    MF_CHECKED, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_HOTKEY,
    WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};

const ID_TRAY: u32 = 1;
const WM_TRAY: u32 = WM_USER + 1;
const HOTKEY_COLOR: i32 = 100;
const HOTKEY_TEXT: i32 = 101;
const MENU_AUTOSTART: usize = 200;
const MENU_EXIT: usize = 201;

static AUTOSTART: AtomicBool = AtomicBool::new(false);

pub fn run() {
    unsafe {
        if let Ok(_mutex) = CreateMutexW(None, false, w!("QuietToolsMutex")) {
            if GetLastError() == ERROR_ALREADY_EXISTS {
                return;
            }
        }
    }

    crate::screen::enable_dpi_awareness();
    crate::window_manager::start();

    unsafe {
        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        let class_name = w!("QuietToolsMainWindow");
        let icon = LoadIconW(Some(hinstance.into()), PCWSTR(101usize as *const _)).unwrap_or_default();
        let wc = WNDCLASSW {
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: icon,
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            lpfnWndProc: Some(wnd_proc),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!("QuietTools"),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance.into()),
            None,
        );

        if hwnd.is_err() {
            return;
        }
        let hwnd = hwnd.unwrap();

        if RegisterHotKey(Some(hwnd), HOTKEY_COLOR, MOD_CONTROL | MOD_SHIFT, 'C' as u32).is_err() {
            show_error("Failed to register Ctrl+Shift+C hotkey (possibly already in use by another app)");
        }
        if RegisterHotKey(Some(hwnd), HOTKEY_TEXT, MOD_CONTROL | MOD_ALT, 'C' as u32).is_err() {
            show_error("Failed to register Ctrl+Alt+C hotkey (possibly already in use by another app)");
        }
        add_tray(hwnd);
        AUTOSTART.store(crate::app::is_autostart_enabled(), Ordering::Relaxed);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let _ = lparam.0 as *const CREATESTRUCTW;
            LRESULT(0)
        }
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_COLOR => crate::overlay::open_color_picker(),
                HOTKEY_TEXT => crate::overlay::open_text_extractor(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TRAY if lparam.0 as u32 == WM_RBUTTONUP => {
            show_tray_menu(hwnd);
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 & 0xffff {
                MENU_AUTOSTART => {
                    let enabled = !AUTOSTART.load(Ordering::Relaxed);
                    if set_autostart(enabled).is_ok() {
                        AUTOSTART.store(enabled, Ordering::Relaxed);
                    }
                }
                MENU_EXIT => {
                    let _ = DestroyWindow(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            delete_tray(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn add_tray(hwnd: HWND) {
    let nid = tray_data(hwnd);
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

unsafe fn delete_tray(hwnd: HWND) {
    let nid = tray_data(hwnd);
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let icon = LoadIconW(Some(hinst.into()), PCWSTR(101usize as *const _)).unwrap_or_default();
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ID_TRAY,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    write_wide_fixed(&mut nid.szTip, "QuietTools");
    nid
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let menu = CreatePopupMenu().unwrap_or_default();
    let flags = if AUTOSTART.load(Ordering::Relaxed) { MF_STRING | MF_CHECKED } else { MF_STRING };
    let auto_w: Vec<u16> = "Run at startup".encode_utf16().chain(Some(0)).collect();
    let exit_w: Vec<u16> = "Exit".encode_utf16().chain(Some(0)).collect();
    let _ = AppendMenuW(menu, flags, MENU_AUTOSTART, windows::core::PCWSTR(auto_w.as_ptr()));
    let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, windows::core::PCWSTR(exit_w.as_ptr()));
    let mut pt = Default::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), hwnd, None);
    let _ = DestroyMenu(menu);
}

fn write_wide_fixed<const N: usize>(dst: &mut [u16; N], text: &str) {
    for (i, ch) in text.encode_utf16().take(N - 1).enumerate() {
        dst[i] = ch;
    }
}

fn autostart_value() -> Result<(windows::Win32::System::Registry::HKEY, Vec<u16>, Vec<u16>), String> {
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let cmd = format!("\"{}\"", exe.display());
    Ok((
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run".encode_utf16().chain(Some(0)).collect(),
        cmd.encode_utf16().chain(Some(0)).collect(),
    ))
}

fn is_autostart_enabled() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let Ok((root, path, _)) = autostart_value() else { return false };
    let name: Vec<u16> = "QuietTools".encode_utf16().chain(Some(0)).collect();
    unsafe { RegGetValueW(root, windows::core::PCWSTR(path.as_ptr()), windows::core::PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, None, None).is_ok() }
}

fn set_autostart(enabled: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{RegCloseKey, RegCreateKeyW, RegDeleteValueW, RegSetValueExW, HKEY, REG_SZ};
    let (root, path, value) = autostart_value()?;
    let name: Vec<u16> = "QuietTools".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut key = HKEY::default();
        let err = RegCreateKeyW(root, windows::core::PCWSTR(path.as_ptr()), &mut key);
        if err.0 != 0 {
            return Err(format!("RegCreateKeyW failed: {}", err.0));
        }

        if enabled {
            let err = RegSetValueExW(
                key,
                windows::core::PCWSTR(name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2)),
            );
            let _ = RegCloseKey(key);
            if err.0 == 0 { Ok(()) } else { Err(format!("RegSetValueExW failed: {}", err.0)) }
        } else {
            let err = RegDeleteValueW(key, windows::core::PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(key);
            if err.0 == 0 { Ok(()) } else { Err(format!("RegDeleteValueW failed: {}", err.0)) }
        }
    }
}

pub fn install_mode() {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return,
    };
    let local = match std::env::var("LOCALAPPDATA") {
        Ok(v) => v,
        Err(_) => return,
    };
    let dir = local + "\\QuietTools";
    let target = std::path::Path::new(&dir).join("QuietTools.exe");
    let dir_w: Vec<u16> = dir.encode_utf16().chain(Some(0)).collect();
    let exe_s = exe.to_string_lossy().to_string();
    let exe_w: Vec<u16> = exe_s.encode_utf16().chain(Some(0)).collect();
    let target_s = target.to_string_lossy().to_string();
    let target_w: Vec<u16> = target_s.encode_utf16().chain(Some(0)).collect();
    let cmd = format!("\"{}\"", target_s);
    let mut cmd_w: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();

    unsafe {
        use windows::Win32::Storage::FileSystem::{CopyFileW, CreateDirectoryW};
        use windows::Win32::System::Threading::{CreateProcessW, PROCESS_INFORMATION, PROCESS_CREATION_FLAGS, STARTUPINFOW};
        let _ = CreateDirectoryW(PCWSTR(dir_w.as_ptr()), Some(std::ptr::null()));
        if CopyFileW(PCWSTR(exe_w.as_ptr()), PCWSTR(target_w.as_ptr()), false).is_err() {
            return;
        }
        let name: Vec<u16> = "QuietTools".encode_utf16().chain(Some(0)).collect();
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run".encode_utf16().chain(Some(0)).collect();
        let val: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
        };
        let mut hkey = HKEY::default();
        if RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), &mut hkey).0 == 0 {
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(val.as_ptr() as *const u8, val.len() * 2)),
            );
            let _ = RegCloseKey(hkey);
        }
        let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
        let mut pi = PROCESS_INFORMATION::default();
        let _ = CreateProcessW(
            PCWSTR::default(),
            Some(PWSTR(cmd_w.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(0),
            None,
            PCWSTR::default(),
            &si,
            &mut pi,
        );
    }
}

pub fn show_error(text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = MessageBoxW(None, windows::core::PCWSTR(wide.as_ptr()), w!("QuietTools"), Default::default());
    }
}
