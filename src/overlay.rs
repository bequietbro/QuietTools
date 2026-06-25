use std::sync::Arc;
use std::thread;
use std::time::Instant;

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreatePen, CreateSolidBrush,
    DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC, LineTo, MoveToEx,
    SelectObject, SetBkMode, SetTextColor, StretchBlt, BLENDFUNCTION, BI_RGB, BITMAPINFO,
    BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_QUALITY, DIB_RGB_COLORS, DT_CENTER,
    DT_CALCRECT, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, FW_NORMAL, HBITMAP, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS,
    PS_DASH, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SetCapture, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, SetCursor, SetTimer,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, IDC_ARROW, IDC_CROSS, IDC_HAND, MSG, SW_SHOW, WM_APP, WM_CREATE, WM_DESTROY,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_TIMER,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::screen::ScreenCapture;

const POPUP_W: i32 = 280;
const POPUP_H: i32 = 160;
const POPUP_PAD: i32 = 14;
const BUTTON_H: i32 = 34;
const SWATCH_SIZE: i32 = 48;

const ERROR_TIMER_ID: usize = 50;
const ERROR_DURATION_MS: u32 = 2500;

enum Mode {
    Color,
    Text,
}

struct PickedColor {
    r: u8,
    g: u8,
    b: u8,
    px: i32,
    py: i32,
}


struct OverlayState {
    mode: Mode,
    screen: Arc<ScreenCapture>,
    bg: HBITMAP,
    mem_dc: HDC,
    back_bmp: HBITMAP,
    back_dc: HDC,
    cursor: POINT,
    selecting: bool,
    start: POINT,
    current: POINT,
    busy: bool,
    picked: Option<PickedColor>,
    error: Option<(String, Instant)>,
}

unsafe impl Send for OverlayState {}

pub fn open_color_picker() {
    spawn(Mode::Color);
}

pub fn open_text_extractor() {
    spawn(Mode::Text);
}

fn spawn(mode: Mode) {
    thread::spawn(move || {
        if let Err(e) = run_overlay(mode) {
            crate::app::show_error(&e);
        }
    });
}

fn run_overlay(mode: Mode) -> Result<(), String> {
    let screen = crate::screen::capture_to_cache()?;
    unsafe {
        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        let class = match mode {
            Mode::Color => w!("QuietToolsColorOverlay"),
            Mode::Text => w!("QuietToolsTextOverlay"),
        };
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            hInstance: hinstance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wnd_proc),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let (bg, mem_dc) = create_screen_bitmap(&screen)?;
        let (back_bmp, back_dc) = create_back_buffer(&screen)?;
        let state = Box::new(OverlayState {
            mode,
            cursor: POINT { x: screen.x + screen.width / 2, y: screen.y + screen.height / 2 },
            screen: screen.clone(),
            bg,
            mem_dc,
            back_bmp,
            back_dc,
            selecting: false,
            start: POINT::default(),
            current: POINT::default(),
            busy: false,
            picked: None,
            error: None,
        });
        let raw = Box::into_raw(state);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("QuietTools Overlay"),
            WS_POPUP,
            screen.x,
            screen.y,
            screen.width,
            screen.height,
            None,
            None,
            Some(hinstance.into()),
            Some(raw as *const _),
        )
        .map_err(|e| e.to_string())?;

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), ERROR_TIMER_ID);
            let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut OverlayState;
            if !ptr.is_null() {
                let state = Box::from_raw(ptr);
                let _ = DeleteObject(HGDIOBJ(state.bg.0));
                let _ = DeleteDC(state.mem_dc);
                let _ = DeleteObject(HGDIOBJ(state.back_bmp.0));
                let _ = DeleteDC(state.back_dc);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_ESCAPE.0 {
                if let Some(state) = state(hwnd) {
                    if state.busy {
                        state.busy = false;
                        state.error = Some(("No text found".into(), Instant::now()));
                        let _ = SetTimer(Some(hwnd), ERROR_TIMER_ID, ERROR_DURATION_MS, None);
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                        return LRESULT(0);
                    }
                    if state.error.is_some() {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    }
                    if state.picked.is_some() {
                        state.picked = None;
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                        return LRESULT(0);
                    }
                }
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            if let Some(state) = state(hwnd) {
                if matches!(state.mode, Mode::Color) && state.picked.is_none() {
                    let step = if (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0 { 5 } else { 1 };
                    let (dx, dy) = match vk {
                        _ if vk == VK_LEFT.0 => (-step, 0),
                        _ if vk == VK_RIGHT.0 => (step, 0),
                        _ if vk == VK_UP.0 => (0, -step),
                        _ if vk == VK_DOWN.0 => (0, step),
                        _ => (0, 0),
                    };
                    if dx != 0 || dy != 0 {
                        state.cursor.x = (state.cursor.x + dx).clamp(state.screen.x, state.screen.x + state.screen.width - 1);
                        state.cursor.y = (state.cursor.y + dy).clamp(state.screen.y, state.screen.y + state.screen.height - 1);
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                        return LRESULT(0);
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state(hwnd) {
                state.cursor = point_from_lparam(lparam, state.screen.x, state.screen.y);
                if state.selecting {
                    state.current = state.cursor;
                }
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = state(hwnd) {
                match state.mode {
                    Mode::Color => {
                        if state.picked.is_some() {
                            handle_popup_click(hwnd, state, lparam);
                        } else {
                            select_color(hwnd, state);
                        }
                    }
                    Mode::Text => {
                        state.error = None;
                        state.selecting = true;
                        state.start = point_from_lparam(lparam, state.screen.x, state.screen.y);
                        state.current = state.start;
                        let _ = SetCapture(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = state(hwnd) {
                if matches!(state.mode, Mode::Text) && state.selecting && !state.busy {
                    state.selecting = false;
                    state.current = point_from_lparam(lparam, state.screen.x, state.screen.y);
                    finish_text_selection(hwnd, state);
                }
            }
            LRESULT(0)
        }
        WM_APP => {
            if let Some(state) = state(hwnd) {
                state.busy = false;
                let result_ptr = lparam.0 as *mut String;
                if !result_ptr.is_null() {
                    let result = Box::from_raw(result_ptr);
                    if state.error.is_none() {
                        if wparam.0 != 0 {
                            let _ = crate::clipboard::write_text(&result);
                            let _ = DestroyWindow(hwnd);
                            return LRESULT(0);
                        }
                        state.error = Some((*result, Instant::now()));
                        let _ = SetTimer(Some(hwnd), ERROR_TIMER_ID, ERROR_DURATION_MS, None);
                    }
                }
            }
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == ERROR_TIMER_ID {
                if let Some(state) = state(hwnd) {
                    state.error = None;
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            if let Some(state) = state(hwnd) {
                paint(hwnd, state);
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            if let Some(state) = state(hwnd) {
                if state.picked.is_some() {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let cx = pt.x - state.screen.x;
                    let cy = pt.y - state.screen.y;
                    let (over, on_btn) = popup_hit(state, cx, cy);
                    if on_btn {
                        let _ = SetCursor(Some(LoadCursorW(None, IDC_HAND).unwrap_or_default()));
                        return LRESULT(1);
                    }
                    if over {
                        let _ = SetCursor(Some(LoadCursorW(None, IDC_ARROW).unwrap_or_default()));
                        return LRESULT(1);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut OverlayState> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
    if ptr.is_null() { None } else { Some(&mut *ptr) }
}

unsafe fn paint(hwnd: HWND, state: &mut OverlayState) {
    use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let bdc = state.back_dc;
    let _ = BitBlt(bdc, 0, 0, state.screen.width, state.screen.height, Some(state.mem_dc), 0, 0, SRCCOPY);
    dim(bdc, state.screen.width, state.screen.height, match state.mode { Mode::Color => 64, Mode::Text => 90 });

    let scale = state.screen.scale;
    match state.mode {
        Mode::Color => paint_color(bdc, state, scale),
        Mode::Text => paint_text(bdc, state, scale),
    }
    let _ = BitBlt(hdc, 0, 0, state.screen.width, state.screen.height, Some(bdc), 0, 0, SRCCOPY);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn paint_color(hdc: HDC, state: &mut OverlayState, scale: f64) {
    let local = POINT { x: state.cursor.x - state.screen.x, y: state.cursor.y - state.screen.y };
    let color = color_at(state, state.cursor.x, state.cursor.y);
    let hex = format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2);

    if let Some(ref picked) = state.picked {
        draw_popup(hdc, picked, scale);
    } else {
        let lx = if local.x + 230 > state.screen.width { local.x - 230 } else { local.x + 28 };
        let ly = if local.y + 250 > state.screen.height { local.y - 250 } else { local.y + 28 };
        draw_loupe(hdc, state, local.x, local.y, lx, ly);
        draw_label(hdc, lx, ly + 208, 198, 28, &hex, rgb(18, 18, 20), rgb(255, 255, 255), scale);
    }
}

unsafe fn paint_text(hdc: HDC, state: &OverlayState, scale: f64) {
    if state.selecting {
        let r = selection_rect(state);
        let _ = BitBlt(hdc, r.left, r.top, r.right - r.left, r.bottom - r.top, Some(state.mem_dc), r.left, r.top, SRCCOPY);
        draw_dashed_frame(hdc, r, rgb(255, 255, 255));
    } else if state.busy {
        draw_message_panel(hdc, state.screen.width, state.screen.height, "Recognizing...", rgb(20, 20, 22), rgb(250, 250, 250), scale);
    } else if let Some((ref text, _)) = state.error {
        draw_error_panel(hdc, state.screen.width, state.screen.height, text, scale);
    } else {
        draw_label(hdc, state.screen.width / 2 - 105, state.screen.height - 56, 210, 30, "Drag to select text", rgb(20, 20, 22), rgb(180, 180, 190), scale);
    }
}

unsafe fn dim(hdc: HDC, w: i32, h: i32, alpha: u8) {
    let mem = CreateCompatibleDC(Some(hdc));
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 1,
            biHeight: 1,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = null_mut();
    let bmp = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    if !bits.is_null() {
        *(bits as *mut u32) = 0;
    }
    let old = SelectObject(mem, HGDIOBJ(bmp.0));
    let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: 0 };
    let _ = AlphaBlend(hdc, 0, 0, w, h, mem, 0, 0, 1, 1, blend);
    SelectObject(mem, old);
    let _ = DeleteObject(HGDIOBJ(bmp.0));
    let _ = DeleteDC(mem);
}

unsafe fn draw_loupe(hdc: HDC, state: &OverlayState, cx: i32, cy: i32, x: i32, y: i32) {
    const GRID: i32 = 11;
    const ZOOM: i32 = 18;
    let size = GRID * ZOOM;
    let _ = StretchBlt(hdc, x, y, size, size, Some(state.mem_dc), cx - GRID / 2, cy - GRID / 2, GRID, GRID, SRCCOPY);

    let border_pen = CreatePen(PS_SOLID, 2, rgb(200, 200, 210));
    let grid_pen = CreatePen(PS_SOLID, 1, rgb(180, 180, 190));
    let center_pen = CreatePen(PS_SOLID, 2, rgb(255, 255, 255));
    let old = SelectObject(hdc, HGDIOBJ(border_pen.0));
    let _ = MoveToEx(hdc, x, y, None);
    let _ = LineTo(hdc, x + size, y);
    let _ = LineTo(hdc, x + size, y + size);
    let _ = LineTo(hdc, x, y + size);
    let _ = LineTo(hdc, x, y);

    SelectObject(hdc, HGDIOBJ(grid_pen.0));
    for i in 1..GRID {
        let p = i * ZOOM;
        let _ = MoveToEx(hdc, x + p, y, None);
        let _ = LineTo(hdc, x + p, y + size);
        let _ = MoveToEx(hdc, x, y + p, None);
        let _ = LineTo(hdc, x + size, y + p);
    }

    let c = (GRID / 2) * ZOOM;
    SelectObject(hdc, HGDIOBJ(center_pen.0));
    let _ = MoveToEx(hdc, x + c, y + c, None);
    let _ = LineTo(hdc, x + c + ZOOM, y + c);
    let _ = LineTo(hdc, x + c + ZOOM, y + c + ZOOM);
    let _ = LineTo(hdc, x + c, y + c + ZOOM);
    let _ = LineTo(hdc, x + c, y + c);

    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(border_pen.0));
    let _ = DeleteObject(HGDIOBJ(grid_pen.0));
    let _ = DeleteObject(HGDIOBJ(center_pen.0));
}

fn sf(size: i32, scale: f64) -> i32 {
    (size as f64 * scale).max(1.0) as i32
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_label(hdc: HDC, x: i32, y: i32, w: i32, h: i32, text: &str, bg: COLORREF, fg: COLORREF, scale: f64) {
    let brush = CreateSolidBrush(bg);
    let rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
    FillRect(hdc, &rect, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
    let font = CreateFontW(sf(15, scale), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, 0, w!("Consolas"));
    let old_font = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, fg);
    let mut r = rect;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut wide, &mut r, DT_CENTER | DT_VCENTER | DT_WORDBREAK);
    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

unsafe fn draw_message_panel(hdc: HDC, screen_w: i32, screen_h: i32, text: &str, bg: COLORREF, fg: COLORREF, scale: f64) {
    let max_w = (screen_w - 40).min(500);

    let font = CreateFontW(sf(15, scale), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, 0, w!("Consolas"));
    let old_font = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, fg);

    let mut measure = RECT { left: 0, top: 0, right: max_w, bottom: 0 };
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut wide, &mut measure, DT_CENTER | DT_WORDBREAK | DT_CALCRECT);

    let pad = 10;
    let pw = measure.right + pad * 2;
    let ph = measure.bottom + pad * 2;
    let px = (screen_w - pw) / 2;
    let py = screen_h * 3 / 4 - ph / 2;

    let brush = CreateSolidBrush(bg);
    let rect = RECT { left: px, top: py, right: px + pw, bottom: py + ph };
    FillRect(hdc, &rect, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));

    let mut text_r = RECT { left: px + pad, top: py + pad, right: px + pw - pad, bottom: py + ph - pad };
    let mut wide2: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut wide2, &mut text_r, DT_CENTER | DT_WORDBREAK);

    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

unsafe fn draw_error_panel(hdc: HDC, screen_w: i32, screen_h: i32, text: &str, scale: f64) {
    draw_message_panel(hdc, screen_w, screen_h, text, rgb(127, 29, 29), rgb(254, 226, 226), scale);
}

unsafe fn draw_popup(hdc: HDC, picked: &PickedColor, scale: f64) {
    let x = picked.px;
    let y = picked.py;
    let hex = format!("#{:02X}{:02X}{:02X}", picked.r, picked.g, picked.b);
    let rgb_text = format!("rgb({}, {}, {})", picked.r, picked.g, picked.b);

    let bg_brush = CreateSolidBrush(rgb(18, 18, 20));
    let rect = RECT { left: x, top: y, right: x + POPUP_W, bottom: y + POPUP_H };
    FillRect(hdc, &rect, bg_brush);
    let _ = DeleteObject(HGDIOBJ(bg_brush.0));

    let border_pen = CreatePen(PS_SOLID, 1, rgb(255, 255, 255));
    let old_pen = SelectObject(hdc, HGDIOBJ(border_pen.0));
    let _ = MoveToEx(hdc, x, y, None);
    let _ = LineTo(hdc, x + POPUP_W, y);
    let _ = LineTo(hdc, x + POPUP_W, y + POPUP_H);
    let _ = LineTo(hdc, x, y + POPUP_H);
    let _ = LineTo(hdc, x, y);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(HGDIOBJ(border_pen.0));

    let swatch = CreateSolidBrush(rgb(picked.r, picked.g, picked.b));
    let swatch_r = RECT { left: x + POPUP_PAD, top: y + POPUP_PAD, right: x + POPUP_PAD + SWATCH_SIZE, bottom: y + POPUP_PAD + SWATCH_SIZE };
    FillRect(hdc, &swatch_r, swatch);
    let _ = DeleteObject(HGDIOBJ(swatch.0));

    let font = CreateFontW(sf(14, scale), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, 0, w!("Consolas"));
    let old_font = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, rgb(250, 250, 250));
    let hex_rect = RECT { left: x + POPUP_PAD + SWATCH_SIZE + 10, top: y + POPUP_PAD + 2, right: x + POPUP_W - POPUP_PAD, bottom: y + POPUP_PAD + SWATCH_SIZE / 2 - 2 };
    let mut hex_wide: Vec<u16> = hex.encode_utf16().collect();
    let mut hr = hex_rect;
    DrawTextW(hdc, &mut hex_wide, &mut hr, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    SetTextColor(hdc, rgb(180, 180, 190));
    let rgb_rect = RECT { left: x + POPUP_PAD + SWATCH_SIZE + 10, top: y + POPUP_PAD + SWATCH_SIZE / 2 + 2, right: x + POPUP_W - POPUP_PAD, bottom: y + POPUP_PAD + SWATCH_SIZE };
    let mut rgb_wide: Vec<u16> = rgb_text.encode_utf16().collect();
    let mut rr = rgb_rect;
    DrawTextW(hdc, &mut rgb_wide, &mut rr, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(font.0));

    let btn_y = y + POPUP_PAD + SWATCH_SIZE + 10;
    let btn_w = POPUP_W - POPUP_PAD * 2;
    draw_popup_button(hdc, x + POPUP_PAD, btn_y, btn_w, BUTTON_H, "Copy HEX", scale);
    draw_popup_button(hdc, x + POPUP_PAD, btn_y + BUTTON_H + 6, btn_w, BUTTON_H, "Copy RGB", scale);
}

unsafe fn draw_popup_button(hdc: HDC, x: i32, y: i32, w: i32, h: i32, text: &str, scale: f64) {
    let brush = CreateSolidBrush(rgb(63, 63, 70));
    let rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
    FillRect(hdc, &rect, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
    let font = CreateFontW(sf(13, scale), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, 0, w!("Consolas"));
    let old_font = SelectObject(hdc, HGDIOBJ(font.0));
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, rgb(250, 250, 250));
    let mut r = rect;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    DrawTextW(hdc, &mut wide, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(font.0));
}

unsafe fn handle_popup_click(hwnd: HWND, state: &mut OverlayState, lparam: LPARAM) {
    let mx = (lparam.0 as i16) as i32;
    let my = ((lparam.0 >> 16) as i16) as i32;
    let Some(ref picked) = state.picked else { return };
    let px = picked.px;
    let py = picked.py;
    let btn_w = POPUP_W - POPUP_PAD * 2;
    let btn_y1 = py + POPUP_PAD + SWATCH_SIZE + 10;
    let btn_y2 = btn_y1 + BUTTON_H + 6;

    if mx < px || mx > px + POPUP_W || my < py || my > py + POPUP_H {
        state.picked = None;
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        return;
    }

    let text = if my >= btn_y1 && my < btn_y1 + BUTTON_H && mx >= px + POPUP_PAD && mx < px + POPUP_PAD + btn_w {
        format!("#{:02X}{:02X}{:02X}", picked.r, picked.g, picked.b)
    } else if my >= btn_y2 && my < btn_y2 + BUTTON_H && mx >= px + POPUP_PAD && mx < px + POPUP_PAD + btn_w {
        format!("rgb({}, {}, {})", picked.r, picked.g, picked.b)
    } else {
        return;
    };
    if crate::clipboard::write_text(&text).is_ok() {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe fn create_screen_bitmap(screen: &ScreenCapture) -> Result<(HBITMAP, HDC), String> {
    let dc = GetDC(None);
    if dc.is_invalid() {
        return Err("GetDC failed".into());
    }
    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        crate::screen::release_dc(HWND::default(), dc);
        return Err("CreateCompatibleDC failed".into());
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: screen.width,
            biHeight: -screen.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| e.to_string())?;
    if bits.is_null() {
        let _ = DeleteDC(mem);
        crate::screen::release_dc(HWND::default(), dc);
        return Err("CreateDIBSection failed".into());
    }
    let count = (screen.width as usize) * (screen.height as usize);
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, count * 4);
    dst.copy_from_slice(&screen.pixels);
    SelectObject(mem, HGDIOBJ(bmp.0));
    crate::screen::release_dc(HWND::default(), dc);
    Ok((bmp, mem))
}

unsafe fn create_back_buffer(screen: &ScreenCapture) -> Result<(HBITMAP, HDC), String> {
    let dc = GetDC(None);
    if dc.is_invalid() {
        return Err("GetDC failed".into());
    }
    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        crate::screen::release_dc(HWND::default(), dc);
        return Err("CreateCompatibleDC failed".into());
    }
    let bmp = CreateCompatibleBitmap(dc, screen.width, screen.height);
    if bmp.is_invalid() {
        let _ = DeleteDC(mem);
        crate::screen::release_dc(HWND::default(), dc);
        return Err("CreateCompatibleBitmap failed".into());
    }
    SelectObject(mem, HGDIOBJ(bmp.0));
    crate::screen::release_dc(HWND::default(), dc);
    Ok((bmp, mem))
}

fn point_from_lparam(lparam: LPARAM, sx: i32, sy: i32) -> POINT {
    let x = (lparam.0 as i16) as i32;
    let y = ((lparam.0 >> 16) as i16) as i32;
    POINT { x: sx + x, y: sy + y }
}

fn color_at(state: &OverlayState, x: i32, y: i32) -> (u8, u8, u8) {
    let lx = (x - state.screen.x).clamp(0, state.screen.width - 1);
    let ly = (y - state.screen.y).clamp(0, state.screen.height - 1);
    let idx = ((ly * state.screen.width + lx) as usize) * 4;
    (state.screen.pixels[idx + 2], state.screen.pixels[idx + 1], state.screen.pixels[idx])
}

unsafe fn draw_dashed_frame(hdc: HDC, r: RECT, color: COLORREF) {
    let pen = CreatePen(PS_DASH, 1, color);
    let old = SelectObject(hdc, HGDIOBJ(pen.0));
    let _ = MoveToEx(hdc, r.left, r.top, None);
    let _ = LineTo(hdc, r.right, r.top);
    let _ = LineTo(hdc, r.right, r.bottom);
    let _ = LineTo(hdc, r.left, r.bottom);
    let _ = LineTo(hdc, r.left, r.top);
    SelectObject(hdc, old);
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

fn popup_hit(state: &OverlayState, mx: i32, my: i32) -> (bool, bool) {
    let Some(ref picked) = state.picked else { return (false, false) };
    let over = mx >= picked.px && mx < picked.px + POPUP_W && my >= picked.py && my < picked.py + POPUP_H;
    if !over { return (false, false); }
    let btn_w = POPUP_W - POPUP_PAD * 2;
    let btn_y1 = picked.py + POPUP_PAD + SWATCH_SIZE + 10;
    let btn_y2 = btn_y1 + BUTTON_H + 6;
    let on_btn = (my >= btn_y1 && my < btn_y1 + BUTTON_H && mx >= picked.px + POPUP_PAD && mx < picked.px + POPUP_PAD + btn_w)
              || (my >= btn_y2 && my < btn_y2 + BUTTON_H && mx >= picked.px + POPUP_PAD && mx < picked.px + POPUP_PAD + btn_w);
    (true, on_btn)
}

fn selection_rect(state: &OverlayState) -> RECT {
    let x1 = state.start.x.min(state.current.x) - state.screen.x;
    let y1 = state.start.y.min(state.current.y) - state.screen.y;
    let x2 = state.start.x.max(state.current.x) - state.screen.x;
    let y2 = state.start.y.max(state.current.y) - state.screen.y;
    RECT { left: x1, top: y1, right: x2, bottom: y2 }
}

unsafe fn select_color(hwnd: HWND, state: &mut OverlayState) {
    let (r, g, b) = color_at(state, state.cursor.x, state.cursor.y);
    let vw = state.screen.width;
    let vh = state.screen.height;
    let local_x = state.cursor.x - state.screen.x;
    let local_y = state.cursor.y - state.screen.y;

    let px = if local_x + 28 + POPUP_W > vw { local_x - POPUP_W - 28 } else { local_x + 28 };
    let py = if local_y + 28 + POPUP_H > vh { vh - POPUP_H - 10 } else { local_y + 28 };
    let px = px.clamp(10, vw - POPUP_W - 10);
    let py = py.clamp(10, vh - POPUP_H - 10);

    state.picked = Some(PickedColor { r, g, b, px, py });
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
}

unsafe fn finish_text_selection(hwnd: HWND, state: &mut OverlayState) {
    let r = selection_rect(state);
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    if w < 5 || h < 5 {
        let _ = DestroyWindow(hwnd);
        return;
    }
    state.busy = true;
    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    let x = r.left + state.screen.x;
    let y = r.top + state.screen.y;
    if let Some((cw, ch, crop)) = crate::screen::crop_region(x, y, w, h) {
        let raw_hwnd = hwnd.0 as isize;
        std::thread::spawn(move || {
            let hwnd = HWND(raw_hwnd as *mut _);
            let result = crate::ocr::recognize_region(&crop, cw, ch);
            let (success, msg) = match result {
                Ok(ref text) if !text.trim().is_empty() => (1usize, text.clone()),
                Err(ref e) => (0usize, e.clone()),
                _ => (0usize, "No text found".into()),
            };
            let boxed = Box::into_raw(Box::new(msg));
            let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(success), LPARAM(boxed as isize));
        });
    } else {
        state.busy = false;
        state.error = Some(("Failed to capture region".into(), Instant::now()));
        let _ = SetTimer(Some(hwnd), ERROR_TIMER_ID, ERROR_DURATION_MS, None);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

use std::ptr::null_mut;

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}
