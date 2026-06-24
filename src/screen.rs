use std::sync::{Arc, Mutex, OnceLock};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

#[derive(Clone)]
pub struct ScreenCapture {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    pub pixels: Vec<u8>,
}

static SCREEN_CACHE: OnceLock<Mutex<Option<Arc<ScreenCapture>>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Arc<ScreenCapture>>> {
    SCREEN_CACHE.get_or_init(|| Mutex::new(None))
}

pub fn enable_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn capture_to_cache() -> Result<Arc<ScreenCapture>, String> {
    let cap = Arc::new(capture_blocking()?);
    *cache().lock().map_err(|e| e.to_string())? = Some(cap.clone());
    Ok(cap)
}

pub fn crop_region(x: i32, y: i32, w: i32, h: i32) -> Option<(u32, u32, Vec<u8>)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let guard = cache().lock().ok()?;
    let cap = guard.as_ref()?;
    let lx = x - cap.x;
    let ly = y - cap.y;
    if lx < 0 || ly < 0 || lx + w > cap.width || ly + h > cap.height {
        return None;
    }

    let mut crop = vec![0u8; (w as usize) * (h as usize) * 4];
    for row in 0..h {
        let src_off = (((ly + row) * cap.width + lx) as usize) * 4;
        let dst_off = (row as usize) * (w as usize) * 4;
        let len = (w as usize) * 4;
        crop[dst_off..dst_off + len].copy_from_slice(&cap.pixels[src_off..src_off + len]);
    }
    Some((w as u32, h as u32, crop))
}

pub fn compute_scale() -> f64 {
    use windows::Win32::Graphics::Gdi::{GetDeviceCaps, LOGPIXELSX};
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return 1.0;
        }
        let dpi = GetDeviceCaps(Some(dc), LOGPIXELSX);
        let _ = ReleaseDC(None, dc);
        if dpi <= 0 { 1.0 } else { dpi as f64 / 96.0 }
    }
}

fn capture_blocking() -> Result<ScreenCapture, String> {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return Err("Invalid screen size".into());
        }

        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return Err("CreateCompatibleDC failed".into());
        }
        let bmp: HBITMAP = CreateCompatibleBitmap(screen_dc, w, h);
        if bmp.is_invalid() {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err("CreateCompatibleBitmap failed".into());
        }

        let old = SelectObject(mem_dc, HGDIOBJ(bmp.0));
        if BitBlt(mem_dc, 0, 0, w, h, Some(screen_dc), x, y, SRCCOPY).is_err() {
            SelectObject(mem_dc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err("BitBlt failed".into());
        }

        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            ..Default::default()
        };

        let lines = GetDIBits(
            mem_dc,
            bmp,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        if lines == 0 {
            return Err("GetDIBits failed".into());
        }

        for chunk in pixels.chunks_exact_mut(4) {
            chunk[3] = 255;
        }

        Ok(ScreenCapture { x, y, width: w, height: h, scale: compute_scale(), pixels })
    }
}

pub fn release_dc(hwnd: HWND, hdc: windows::Win32::Graphics::Gdi::HDC) {
    unsafe {
        let _ = ReleaseDC(Some(hwnd), hdc);
    }
}
