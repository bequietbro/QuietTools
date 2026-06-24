use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetCursorPos, GetLayeredWindowAttributes, GetMessageW, GetWindowLongPtrW,
    GetWindowLongW, GetWindowRect, IsZoomed, LoadCursorW, MoveWindow, SetCursor, SetForegroundWindow,
    SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, ShowWindow, TranslateMessage,
    UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, IDC_ARROW, IDC_SIZEALL,
    IDC_SIZENESW, IDC_SIZENWSE, KBDLLHOOKSTRUCT, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, MSG, MSLLHOOKSTRUCT,
    SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WINDOW_STYLE, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_CAPTION, WS_EX_LAYERED,
    WS_MAXIMIZEBOX, WS_THICKFRAME,
};

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static SHIFT_DOWN: AtomicBool = AtomicBool::new(false);
static DID_USE_WIN: AtomicBool = AtomicBool::new(false);

const OP_MOVE: u8 = 1;
const OP_RESIZE: u8 = 2;
const OP_SNAP: u8 = 3;

static OPERATION: AtomicU8 = AtomicU8::new(0);

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
static ANCHOR_MX: AtomicI32 = AtomicI32::new(0);
static ANCHOR_MY: AtomicI32 = AtomicI32::new(0);
static ANCHOR_WX: AtomicI32 = AtomicI32::new(0);
static ANCHOR_WY: AtomicI32 = AtomicI32::new(0);
static ANCHOR_WW: AtomicI32 = AtomicI32::new(0);
static ANCHOR_WH: AtomicI32 = AtomicI32::new(0);
static ANCHOR_MAXIMIZED: AtomicBool = AtomicBool::new(false);
static RESIZE_CORNER: AtomicU8 = AtomicU8::new(0);

static SNAP_START_ZONE: AtomicU8 = AtomicU8::new(0);
static SNAP_LAST_ZONE: AtomicU8 = AtomicU8::new(0);
static SNAP_APPLIED: AtomicBool = AtomicBool::new(false);

static LAST_CLICK: OnceLock<std::sync::Mutex<Option<Instant>>> = OnceLock::new();
static LAST_CLICK_HWND: AtomicIsize = AtomicIsize::new(0);

const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LSHIFT_U32: u32 = 0xA0;
const VK_RSHIFT_U32: u32 = 0xA1;
const VK_SHIFT_U32: u32 = 0x10;
const DUMMY_KEY: u16 = 0xA0;
const OPACITY_STEP: i32 = 16;
const OPACITY_MIN: i32 = 32;
const DOUBLE_CLICK_MS: u128 = 400;
const MIN_W: i32 = 120;
const MIN_H: i32 = 80;
const SWP_ASYNCWINDOWPOS: u32 = 0x0040;

static CURSOR_KIND: AtomicI32 = AtomicI32::new(-1);
const CUR_DEFAULT: i32 = -1;
const CUR_MOVE: i32 = 0;
const CUR_NWSE: i32 = 1;
const CUR_NESW: i32 = 2;

unsafe fn apply_cursor(kind: i32) {
    let prev = CURSOR_KIND.swap(kind, Ordering::Relaxed);
    if prev == kind { return; }
    let idc = match kind {
        CUR_MOVE => IDC_SIZEALL,
        CUR_NWSE => IDC_SIZENWSE,
        CUR_NESW => IDC_SIZENESW,
        _ => IDC_ARROW,
    };
    if let Ok(hcur) = LoadCursorW(Some(HINSTANCE::default()), idc) {
        SetCursor(Some(hcur));
    }
}

pub fn start() {
    let _ = thread::Builder::new().name("quiettools-winmgr".into()).spawn(hook_thread);
}

fn hook_thread() {
    unsafe {
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(ll_mouse), None, 0).unwrap_or_default();
        let kb_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_keyboard), None, 0).unwrap_or_default();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if !mouse_hook.is_invalid() { let _ = UnhookWindowsHookEx(mouse_hook); }
        if !kb_hook.is_invalid() { let _ = UnhookWindowsHookEx(kb_hook); }
    }
}

unsafe fn win_physically_down() -> bool {
    (GetAsyncKeyState(VK_LWIN as i32) as u16 & 0x8000) != 0 || (GetAsyncKeyState(VK_RWIN as i32) as u16 & 0x8000) != 0
}

unsafe fn sync_win_state() {
    if WIN_DOWN.load(Ordering::Relaxed) && !win_physically_down() {
        WIN_DOWN.store(false, Ordering::Relaxed);
    }
}

unsafe extern "system" fn ll_keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        sync_win_state();
        let kb = &*(l.0 as *const KBDLLHOOKSTRUCT);
        let vk = kb.vkCode;
        let msg = w.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if vk == VK_LWIN || vk == VK_RWIN {
            if down {
                let was_down = WIN_DOWN.swap(true, Ordering::Relaxed);
                if !was_down { DID_USE_WIN.store(false, Ordering::Relaxed); }
            } else if up {
                WIN_DOWN.store(false, Ordering::Relaxed);
                if OPERATION.load(Ordering::Relaxed) != 0 { apply_cursor(CUR_DEFAULT); }
                OPERATION.store(0, Ordering::Relaxed);
                SNAP_LAST_ZONE.store(0, Ordering::Relaxed);
                SNAP_START_ZONE.store(0, Ordering::Relaxed);
                SNAP_APPLIED.store(false, Ordering::Relaxed);
                if DID_USE_WIN.swap(false, Ordering::Relaxed) { inject_dummy_key(); }
            }
        } else if vk == VK_LSHIFT_U32 || vk == VK_RSHIFT_U32 || vk == VK_SHIFT_U32 {
            if down { SHIFT_DOWN.store(true, Ordering::Relaxed); } else if up { SHIFT_DOWN.store(false, Ordering::Relaxed); }
        }
    }
    CallNextHookEx(None, code, w, l)
}

unsafe extern "system" fn ll_mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 { return CallNextHookEx(None, code, w, l); }
    sync_win_state();
    let ms = &*(l.0 as *const MSLLHOOKSTRUCT);
    let pt = ms.pt;
    let msg = w.0 as u32;
    let win = WIN_DOWN.load(Ordering::Relaxed);
    let shift = SHIFT_DOWN.load(Ordering::Relaxed);

    if win {
        match msg {
            WM_LBUTTONDOWN => {
                if let Some(hwnd) = top_level_at(pt) {
                    DID_USE_WIN.store(true, Ordering::Relaxed);
                    if is_double_click(hwnd) { toggle_maximize(hwnd); return LRESULT(1); }
                    let _ = SetForegroundWindow(hwnd);
                    store_anchor(hwnd, pt);
                    if shift {
                        let (start_zone, _) = classify_zone(pt);
                        SNAP_START_ZONE.store(start_zone, Ordering::Relaxed);
                        SNAP_LAST_ZONE.store(start_zone, Ordering::Relaxed);
                        SNAP_APPLIED.store(false, Ordering::Relaxed);
                        OPERATION.store(OP_SNAP, Ordering::Relaxed);
                    } else {
                        OPERATION.store(OP_MOVE, Ordering::Relaxed);
                    }
                    apply_cursor(CUR_MOVE);
                    thread::spawn(move || operation_thread());
                    return LRESULT(1);
                }
            }
            WM_RBUTTONDOWN => {
                if let Some(hwnd) = top_level_at(pt) {
                    if !is_resizable(hwnd) { return CallNextHookEx(None, code, w, l); }
                    DID_USE_WIN.store(true, Ordering::Relaxed);
                    if IsZoomed(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_RESTORE); }
                    let _ = SetForegroundWindow(hwnd);
                    store_anchor(hwnd, pt);
                    let mut r = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut r);
                    let corner = match ((pt.x - r.left) < (r.right - pt.x), (pt.y - r.top) < (r.bottom - pt.y)) {
                        (true, true) => 0u8, (false, true) => 1, (false, false) => 2, (true, false) => 3,
                    };
                    RESIZE_CORNER.store(corner, Ordering::Relaxed);
                    OPERATION.store(OP_RESIZE, Ordering::Relaxed);
                    apply_cursor(match corner { 0 | 2 => CUR_NWSE, _ => CUR_NESW });
                    thread::spawn(move || operation_thread());
                    return LRESULT(1);
                }
            }
            WM_MBUTTONDOWN => {
                if let Some(hwnd) = top_level_at(pt) {
                    DID_USE_WIN.store(true, Ordering::Relaxed);
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    return LRESULT(1);
                }
            }
            WM_MOUSEWHEEL => {
                if let Some(hwnd) = top_level_at(pt) {
                    DID_USE_WIN.store(true, Ordering::Relaxed);
                    let delta = ((ms.mouseData >> 16) & 0xFFFF) as i16;
                    adjust_opacity(hwnd, if delta > 0 { OPACITY_STEP } else { -OPACITY_STEP });
                    return LRESULT(1);
                }
            }
            _ => {}
        }
    }

    match OPERATION.load(Ordering::Relaxed) {
        OP_MOVE if msg == WM_LBUTTONUP => { DID_USE_WIN.store(true, Ordering::Relaxed); OPERATION.store(0, Ordering::Relaxed); apply_cursor(CUR_DEFAULT); return LRESULT(1); }
        OP_SNAP if msg == WM_LBUTTONUP => { DID_USE_WIN.store(true, Ordering::Relaxed); OPERATION.store(0, Ordering::Relaxed); SNAP_LAST_ZONE.store(0, Ordering::Relaxed); SNAP_START_ZONE.store(0, Ordering::Relaxed); SNAP_APPLIED.store(false, Ordering::Relaxed); apply_cursor(CUR_DEFAULT); return LRESULT(1); }
        OP_RESIZE if msg == WM_RBUTTONUP => { DID_USE_WIN.store(true, Ordering::Relaxed); OPERATION.store(0, Ordering::Relaxed); apply_cursor(CUR_DEFAULT); return LRESULT(1); }
        _ => {}
    }

    if msg == WM_MBUTTONUP && win { DID_USE_WIN.store(true, Ordering::Relaxed); return LRESULT(1); }
    CallNextHookEx(None, code, w, l)
}

fn operation_thread() {
    unsafe {
        let op = OPERATION.load(Ordering::Relaxed);
        match op {
            OP_MOVE => run_move(),
            OP_RESIZE => run_resize(),
            OP_SNAP => run_snap(),
            _ => {}
        }
    }
}

unsafe fn run_move() {
    while OPERATION.load(Ordering::Relaxed) == OP_MOVE {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let Some(hwnd) = target() else { break };
        if IsZoomed(hwnd).as_bool() {
            let cur_mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let pt_mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            if cur_mon != pt_mon && !pt_mon.is_invalid() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                if let Some(work) = monitor_workarea(pt_mon) {
                    let _ = MoveWindow(hwnd, work.left, work.top, work.right - work.left, work.bottom - work.top, true);
                }
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
                store_anchor(hwnd, pt);
            }
        } else {
            let dx = pt.x - ANCHOR_MX.load(Ordering::Relaxed);
            let dy = pt.y - ANCHOR_MY.load(Ordering::Relaxed);
            set_pos(hwnd, ANCHOR_WX.load(Ordering::Relaxed) + dx, ANCHOR_WY.load(Ordering::Relaxed) + dy, ANCHOR_WW.load(Ordering::Relaxed), ANCHOR_WH.load(Ordering::Relaxed));
        }
        thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn run_resize() {
    while OPERATION.load(Ordering::Relaxed) == OP_RESIZE {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let Some(hwnd) = target() else { break };
        let dx = pt.x - ANCHOR_MX.load(Ordering::Relaxed);
        let dy = pt.y - ANCHOR_MY.load(Ordering::Relaxed);
        let wx = ANCHOR_WX.load(Ordering::Relaxed);
        let wy = ANCHOR_WY.load(Ordering::Relaxed);
        let ww = ANCHOR_WW.load(Ordering::Relaxed);
        let wh = ANCHOR_WH.load(Ordering::Relaxed);
        let corner = RESIZE_CORNER.load(Ordering::Relaxed);
        let mut x = wx; let mut y = wy; let mut w = ww; let mut h = wh;
        match corner { 0 => { x += dx; y += dy; w -= dx; h -= dy; }, 1 => { y += dy; w += dx; h -= dy; }, 2 => { w += dx; h += dy; }, 3 => { x += dx; w -= dx; h += dy; }, _ => {} }
        if w < MIN_W { if corner == 0 || corner == 3 { x -= MIN_W - w; } w = MIN_W; }
        if h < MIN_H { if corner == 0 || corner == 1 { y -= MIN_H - h; } h = MIN_H; }
        set_pos(hwnd, x, y, w, h);
        thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn run_snap() {
    while OPERATION.load(Ordering::Relaxed) == OP_SNAP {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let Some(hwnd) = target() else { break };
        let (cur_zone, rect_opt) = classify_zone(pt);
        let prev = SNAP_LAST_ZONE.load(Ordering::Relaxed);
        let start = SNAP_START_ZONE.load(Ordering::Relaxed);
        if cur_zone == prev { thread::sleep(Duration::from_millis(1)); continue; }
        let stay = cur_zone == 0 || cur_zone == 5 || cur_zone == start;
        if stay { restore_to_anchor(hwnd); SNAP_LAST_ZONE.store(cur_zone, Ordering::Relaxed); thread::sleep(Duration::from_millis(1)); continue; }
        let Some(r) = rect_opt else { thread::sleep(Duration::from_millis(1)); continue };
        if IsZoomed(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_RESTORE); }
        set_pos(hwnd, r.left, r.top, r.right - r.left, r.bottom - r.top);
        SNAP_APPLIED.store(true, Ordering::Relaxed);
        SNAP_LAST_ZONE.store(cur_zone, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn inject_dummy_key() {
    let inputs = [
        INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(DUMMY_KEY), wScan: 0, dwFlags: KEYBD_EVENT_FLAGS(0), time: 0, dwExtraInfo: 0 } } },
        INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(DUMMY_KEY), wScan: 0, dwFlags: KEYEVENTF_KEYUP, time: 0, dwExtraInfo: 0 } } },
    ];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
}

unsafe fn top_level_at(pt: POINT) -> Option<HWND> {
    let h = WindowFromPoint(pt);
    if h.is_invalid() { return None; }
    let root = GetAncestor(h, GA_ROOT);
    if root.is_invalid() { None } else { Some(root) }
}

unsafe fn store_anchor(hwnd: HWND, pt: POINT) {
    let mut r = RECT::default();
    if GetWindowRect(hwnd, &mut r).is_err() { return; }
    TARGET_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
    ANCHOR_MX.store(pt.x, Ordering::Relaxed);
    ANCHOR_MY.store(pt.y, Ordering::Relaxed);
    ANCHOR_WX.store(r.left, Ordering::Relaxed);
    ANCHOR_WY.store(r.top, Ordering::Relaxed);
    ANCHOR_WW.store(r.right - r.left, Ordering::Relaxed);
    ANCHOR_WH.store(r.bottom - r.top, Ordering::Relaxed);
    ANCHOR_MAXIMIZED.store(IsZoomed(hwnd).as_bool(), Ordering::Relaxed);
}

unsafe fn target() -> Option<HWND> {
    let h = TARGET_HWND.load(Ordering::Relaxed);
    if h == 0 { None } else { Some(HWND(h as *mut _)) }
}

unsafe fn set_pos(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SET_WINDOW_POS_FLAGS(SWP_ASYNCWINDOWPOS));
}

unsafe fn classify_zone(pt: POINT) -> (u8, Option<RECT>) {
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    if mon.is_invalid() { return (0, None); }
    let Some(work) = monitor_workarea(mon) else { return (0, None) };
    let mw = work.right - work.left;
    let mh = work.bottom - work.top;
    if mw <= 0 || mh <= 0 { return (0, None); }
    let col_w = (mw / 3).max(1);
    let row_h = (mh / 3).max(1);
    let rx = (pt.x - work.left).clamp(0, mw - 1);
    let ry = (pt.y - work.top).clamp(0, mh - 1);
    let col = (rx / col_w).min(2);
    let row = (ry / row_h).min(2);
    let cell = (row * 3 + col + 1) as u8;
    let half_w = mw / 2;
    let half_h = mh / 2;
    let rect = match cell { 1 => RECT { left: work.left, top: work.top, right: work.left + half_w, bottom: work.top + half_h }, 2 => RECT { left: work.left, top: work.top, right: work.right, bottom: work.top + half_h }, 3 => RECT { left: work.left + half_w, top: work.top, right: work.right, bottom: work.top + half_h }, 4 => RECT { left: work.left, top: work.top, right: work.left + half_w, bottom: work.bottom }, 5 => RECT::default(), 6 => RECT { left: work.left + half_w, top: work.top, right: work.right, bottom: work.bottom }, 7 => RECT { left: work.left, top: work.top + half_h, right: work.left + half_w, bottom: work.bottom }, 8 => RECT { left: work.left, top: work.top + half_h, right: work.right, bottom: work.bottom }, 9 => RECT { left: work.left + half_w, top: work.top + half_h, right: work.right, bottom: work.bottom }, _ => return (0, None), };
    if cell == 5 { (cell, None) } else { (cell, Some(rect)) }
}

unsafe fn restore_to_anchor(hwnd: HWND) {
    if !SNAP_APPLIED.load(Ordering::Relaxed) { return; }
    if ANCHOR_MAXIMIZED.load(Ordering::Relaxed) {
        if !IsZoomed(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_MAXIMIZE); }
    } else {
        if IsZoomed(hwnd).as_bool() { let _ = ShowWindow(hwnd, SW_RESTORE); }
        set_pos(hwnd, ANCHOR_WX.load(Ordering::Relaxed), ANCHOR_WY.load(Ordering::Relaxed), ANCHOR_WW.load(Ordering::Relaxed), ANCHOR_WH.load(Ordering::Relaxed));
    }
    SNAP_APPLIED.store(false, Ordering::Relaxed);
}

unsafe fn monitor_workarea(mon: HMONITOR) -> Option<RECT> {
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(mon, &mut info).as_bool() { Some(info.rcWork) } else { None }
}

unsafe fn is_resizable(hwnd: HWND) -> bool {
    let style = WINDOW_STYLE(GetWindowLongW(hwnd, GWL_STYLE) as u32);
    style.contains(WS_THICKFRAME) || (style.contains(WS_CAPTION) && style.contains(WS_MAXIMIZEBOX))
}

unsafe fn toggle_maximize(hwnd: HWND) {
    let _ = if IsZoomed(hwnd).as_bool() { ShowWindow(hwnd, SW_RESTORE) } else { ShowWindow(hwnd, SW_MAXIMIZE) };
}

unsafe fn adjust_opacity(hwnd: HWND, delta: i32) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let layered_bit = WS_EX_LAYERED.0 as isize;
    let current = if ex & layered_bit == 0 { let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | layered_bit); let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA); 255i32 } else { read_alpha(hwnd).unwrap_or(255) as i32 };
    let next = (current + delta).clamp(OPACITY_MIN, 255);
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), next as u8, LWA_ALPHA);
    if next >= 255 { let new_ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & !layered_bit; let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex); }
}

unsafe fn read_alpha(hwnd: HWND) -> Option<u8> {
    let mut alpha: u8 = 255;
    let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
    let mut key = COLORREF(0);
    if GetLayeredWindowAttributes(hwnd, Some(&mut key), Some(&mut alpha), Some(&mut flags)).is_ok() { Some(alpha) } else { None }
}

unsafe fn is_double_click(hwnd: HWND) -> bool {
    let mu = LAST_CLICK.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = mu.lock().unwrap();
    let now = Instant::now();
    let prev_hwnd = LAST_CLICK_HWND.load(Ordering::Relaxed);
    let is_dbl = match *guard { Some(prev) => now.duration_since(prev).as_millis() < DOUBLE_CLICK_MS && prev_hwnd == hwnd.0 as isize, None => false };
    if is_dbl { *guard = None; LAST_CLICK_HWND.store(0, Ordering::Relaxed); } else { *guard = Some(now); LAST_CLICK_HWND.store(hwnd.0 as isize, Ordering::Relaxed); }
    is_dbl
}
