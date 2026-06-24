use windows::core::Interface;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;
use windows_future::AsyncStatus;

fn is_confusable(c: char) -> bool {
    matches!(
        c,
        '\u{0430}' | '\u{0435}' | '\u{043C}' | '\u{043E}' | '\u{0440}' | '\u{0441}' | '\u{0443}' | '\u{0445}' | '\u{0456}' |
        '\u{00E0}' | '\u{00E1}' | '\u{00E2}' | '\u{00E3}' | '\u{00E4}' | '\u{00E5}' | '\u{00E8}' | '\u{00E9}' |
        '\u{00EC}' | '\u{00ED}' | '\u{00F2}' | '\u{00F3}' | '\u{00F9}' | '\u{00FA}' | '\u{00FC}' |
        '\u{0101}' | '\u{0113}' | '\u{012B}' | '\u{014D}' | '\u{016B}' |
        '\u{1E01}' | '\u{1E03}' | '\u{1E0B}' | '\u{1E0D}' | '\u{1E1F}' | '\u{1E21}' | '\u{1E37}' | '\u{1E39}' | '\u{1E4B}' | '\u{1E55}' | '\u{1E57}' | '\u{1E59}' | '\u{1E5B}' | '\u{1E5D}' | '\u{1E5F}' | '\u{1E7D}' | '\u{1E81}' | '\u{1E83}' | '\u{1E85}' | '\u{1E8B}' | '\u{1E8D}' | '\u{1E91}' | '\u{1E93}' | '\u{1E95}' |
        '\u{1EA1}' | '\u{1EB9}' | '\u{1ECB}' | '\u{1ED9}' | '\u{1EE9}' | '\u{1EF3}' | '\u{1EF5}' | '\u{1EF7}' | '\u{1EF9}'
    )
}

fn has_genuine_non_ascii(s: &str) -> bool {
    let total = s.chars().count();
    if total == 0 {
        return false;
    }
    let genuine = s.chars().filter(|c| *c > '\u{007F}' && !is_confusable(*c)).count();
    genuine as f64 / total as f64 >= 0.15
}

fn has_non_ascii(s: &str) -> bool {
    s.chars().any(|c| c > '\u{007F}')
}

fn to_grayscale(bgra: &mut [u8]) {
    for pixel in bgra.chunks_exact_mut(4) {
        let b = pixel[0] as f32;
        let g = pixel[1] as f32;
        let r = pixel[2] as f32;
        let y = (0.299 * r + 0.587 * g + 0.114 * b).round() as u8;
        pixel[0] = y;
        pixel[1] = y;
        pixel[2] = y;
    }
}

fn upscale(bgra: &[u8], w: u32, h: u32, scale: u32) -> (Vec<u8>, u32, u32) {
    let nw = w * scale;
    let nh = h * scale;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let sy = (y / scale) as usize;
        for x in 0..nw {
            let sx = (x / scale) as usize;
            let si = (sy * w as usize + sx) * 4;
            let di = (y as usize * nw as usize + x as usize) * 4;
            out[di..di + 4].copy_from_slice(&bgra[si..si + 4]);
        }
    }
    (out, nw, nh)
}

fn try_engine(bitmap: &SoftwareBitmap, engine: &OcrEngine) -> Result<String, String> {
    let operation = engine.RecognizeAsync(bitmap).map_err(|_| "OCR failed")?;

    loop {
        let status = operation.Status().map_err(|_| "OCR failed")?;
        if status == AsyncStatus::Completed {
            break;
        }
        if status == AsyncStatus::Error || status == AsyncStatus::Canceled {
            return Err("OCR failed".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let result = operation.GetResults().map_err(|_| "OCR failed")?;
    let text = result.Text().map_err(|_| "OCR failed")?;
    Ok(text.to_string())
}

fn write_bitmap(bgra: &[u8], width: u32, height: u32) -> Result<SoftwareBitmap, String> {
    let bitmap = SoftwareBitmap::CreateWithAlpha(
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(|_| "Bitmap error")?;

    let buffer = bitmap
        .LockBuffer(BitmapBufferAccessMode::Write)
        .map_err(|_| "Bitmap error")?;

    let desc = buffer.GetPlaneDescription(0).map_err(|_| "Bitmap error")?;
    let stride = desc.Stride as usize;
    let start = desc.StartIndex as usize;

    let reference = buffer.CreateReference().map_err(|_| "Bitmap error")?;
    let byte_access: IMemoryBufferByteAccess = reference.cast().map_err(|_| "Bitmap error")?;

    let mut data = std::ptr::null_mut();
    let mut capacity = 0u32;
    unsafe {
        byte_access
            .GetBuffer(&mut data, &mut capacity)
            .map_err(|_| "Bitmap error")?;
    }

    let row_bytes = (width as usize) * 4;
    if (capacity as usize) >= start + (height as usize) * stride
        && bgra.len() >= row_bytes * (height as usize)
    {
        unsafe {
            for row in 0..height as usize {
                std::ptr::copy_nonoverlapping(
                    bgra.as_ptr().add(row * row_bytes),
                    data.add(start + row * stride),
                    row_bytes,
                );
            }
        }
    }

    drop(byte_access);
    drop(reference);
    let _ = buffer.Close();
    drop(buffer);

    SoftwareBitmap::Copy(&bitmap).map_err(|_| "Bitmap error".to_string())
}

fn save_fallback(t: &str, ascii: &mut Option<String>, confusable: &mut Option<String>) {
    if has_non_ascii(t) {
        if confusable.is_none() {
            *confusable = Some(t.to_string());
        }
    } else if ascii.is_none() {
        *ascii = Some(t.to_string());
    }
}

fn fix_ocr_result(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '\u{0445}'
            && i > 0 && i + 1 < n
            && chars[i - 1].is_ascii_digit()
            && chars[i + 1].is_ascii_digit()
        {
            out.push('\u{00D7}');
            i += 1;
            continue;
        }
        if chars[i] == '*'
            && i > 0 && i + 1 < n
            && chars[i - 1].is_ascii_digit()
            && chars[i + 1].is_ascii_digit()
        {
            out.push('\u{00D7}');
            i += 1;
            continue;
        }
        if i + 1 < n
            && chars[i] == '\u{0440}'
            && chars[i + 1] == '\u{0445}'
        {
            let mut j = i;
            while j > 0 && chars[j - 1] == ' ' {
                j -= 1;
            }
            if j > 0 && chars[j - 1].is_ascii_digit() {
                out.push('p');
                out.push('x');
                i += 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn pick_scale(w: u32, h: u32) -> u32 {
    let m = w.max(h);
    if m < 80 { 8 }
    else if m < 160 { 5 }
    else { 3 }
}

pub fn recognize_region(bgra: &[u8], width: u32, height: u32) -> Result<String, String> {
    let mut pixels = bgra.to_vec();
    to_grayscale(&mut pixels);
    let (pixels, rw, rh) = upscale(&pixels, width, height, pick_scale(width, height));

    let bitmap = write_bitmap(&pixels, rw, rh)?;

    let langs = OcrEngine::AvailableRecognizerLanguages()
        .map_err(|_| "OCR: install a language pack")?;

    let lang_count = langs.Size().unwrap_or(0);
    if lang_count == 0 {
        return Err("OCR: install a language pack".into());
    }

    let mut ascii_fallback: Option<String> = None;
    let mut confusable_fallback: Option<String> = None;

    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        if let Ok(text) = try_engine(&bitmap, &engine) {
            let t = text.trim().to_string();
            if !t.is_empty() {
                if has_genuine_non_ascii(&t) {
                    return Ok(fix_ocr_result(&t));
                }
                save_fallback(&t, &mut ascii_fallback, &mut confusable_fallback);
            }
        }
    }

    for lang in &langs {
        if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
            if let Ok(text) = try_engine(&bitmap, &engine) {
                let t = text.trim().to_string();
                if !t.is_empty() {
                    if has_genuine_non_ascii(&t) {
                        return Ok(fix_ocr_result(&t));
                    }
                    save_fallback(&t, &mut ascii_fallback, &mut confusable_fallback);
                }
            }
        }
    }

    if let Some(text) = ascii_fallback.or(confusable_fallback) {
        return Ok(fix_ocr_result(&text));
    }

    Err("No text found".into())
}
