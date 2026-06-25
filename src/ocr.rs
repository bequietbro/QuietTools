use windows::core::Interface;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;
use windows_future::AsyncStatus;

fn normalize_fullwidth(s: &str) -> String {
    s.chars().map(|c| match c {
        '\u{FF08}' => '(',
        '\u{FF09}' => ')',
        '\u{FF3B}' => '[',
        '\u{FF3D}' => ']',
        '\u{FF5B}' => '{',
        '\u{FF5D}' => '}',
        '\u{FF1A}' => ':',
        '\u{FF1B}' => ';',
        '\u{FF0C}' => ',',
        '\u{FF0E}' => '.',
        '\u{FF10}'..='\u{FF19}' => char::from_u32(c as u32 - 0xFF10 + 0x30).unwrap_or(c),
        '\u{FF21}'..='\u{FF3A}' => char::from_u32(c as u32 - 0xFF21 + 0x41).unwrap_or(c),
        '\u{FF41}'..='\u{FF5A}' => char::from_u32(c as u32 - 0xFF41 + 0x61).unwrap_or(c),
        '\u{0410}' => 'A', '\u{0412}' => 'B', '\u{0415}' => 'E',
        '\u{0406}' => 'I', '\u{041A}' => 'K', '\u{041C}' => 'M',
        '\u{041D}' => 'H', '\u{041E}' => 'O', '\u{0420}' => 'P',
        '\u{0421}' => 'C', '\u{0422}' => 'T', '\u{0425}' => 'X',
        '\u{0430}' => 'a', '\u{0435}' => 'e', '\u{0456}' => 'i',
        '\u{043E}' => 'o', '\u{0440}' => 'p', '\u{0441}' => 'c',
        '\u{0443}' => 'y', '\u{0445}' => 'x',
        _ => c,
    }).collect()
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

fn otsu_threshold(bgra: &mut [u8]) {
    let total = (bgra.len() / 4) as u64;
    if total == 0 {
        return;
    }

    let mut hist = [0u64; 256];
    for pixel in bgra.chunks_exact(4) {
        hist[pixel[0] as usize] += 1;
    }

    let mut best = 128u8;
    let mut max_var = 0.0f64;
    let mut w_b = 0u64;
    let mut sum_b: u64 = 0;
    let mut sum: u64 = 0;
    for (i, &count) in hist.iter().enumerate() {
        sum += (i as u64) * count;
    }

    for (t, count) in hist.iter().enumerate() {
        w_b += count;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += (t as u64) * count;
        let mean_b = sum_b as f64 / w_b as f64;
        let mean_f = (sum - sum_b) as f64 / w_f as f64;
        let var = (w_b as f64) * (w_f as f64) * (mean_b - mean_f).powi(2);
        if var > max_var {
            max_var = var;
            best = t as u8;
        }
    }

    let mut black_count = 0u64;
    for pixel in bgra.chunks_exact_mut(4) {
        let v = if pixel[0] <= best { 0u8 } else { 255u8 };
        pixel[0] = v;
        pixel[1] = v;
        pixel[2] = v;
        if v == 0 {
            black_count += 1;
        }
    }

    if black_count > total * 70 / 100 {
        for pixel in bgra.chunks_exact_mut(4) {
            pixel[0] = 255 - pixel[0];
            pixel[1] = 255 - pixel[1];
            pixel[2] = 255 - pixel[2];
        }
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

fn ocr_bitmap(bitmap: &SoftwareBitmap) -> Result<String, String> {
    let mut collected: Vec<String> = Vec::new();

    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        if let Ok(text) = try_engine(bitmap, &engine) {
            let t = text.trim().to_string();
            if !t.is_empty() {
                collected.push(t);
            }
        }
    }

    let langs = OcrEngine::AvailableRecognizerLanguages()
        .map_err(|_| "OCR: install a language pack")?;
    for lang in &langs {
        if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
            if let Ok(text) = try_engine(bitmap, &engine) {
                let t = text.trim().to_string();
                if !t.is_empty() {
                    collected.push(t);
                }
            }
        }
    }

    if collected.is_empty() {
        return Err("No text found".into());
    }

    collected.sort_by(|a, b| {
        let a_norm = normalize_fullwidth(a);
        let b_norm = normalize_fullwidth(b);
        let a_total = a_norm.chars().count();
        let b_total = b_norm.chars().count();
        let a_script = a_norm.chars().filter(|c| !c.is_ascii() && c.is_alphabetic()).count() > a_total / 10;
        let b_script = b_norm.chars().filter(|c| !c.is_ascii() && c.is_alphabetic()).count() > b_total / 10;
        let a_alpha = a_norm.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        let b_alpha = b_norm.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        b_script.cmp(&a_script).then(b_alpha.cmp(&a_alpha))
    });

    let mut best = normalize_fullwidth(&collected[0]);
    let pairs = [('(', ')'), ('[', ']')];
    for r in &collected[1..] {
        let norm = normalize_fullwidth(r);
        for &(open, close) in &pairs {
            if best.contains(close) && !best.contains(open)
                && norm.contains(open) && norm.contains(close)
            {
                if let Some(pos) = norm.find(open) {
                    let next_word: String = norm[pos + 1..].chars()
                        .skip_while(|c| !c.is_ascii_alphanumeric())
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    if !next_word.is_empty() {
                        if let Some(wp) = best.find(&next_word) {
                            let start = wp - best[..wp].chars().rev().take_while(|c| c.is_ascii_alphanumeric()).count();
                            best.insert(start, open);
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(fix_ocr_result(&best))
}

fn threshold_fixed(bgra: &mut [u8]) {
    let total = bgra.len() / 4;
    for p in bgra.chunks_exact_mut(4) {
        let v = if p[0] <= 128 { 0 } else { 255 };
        p[0] = v; p[1] = v; p[2] = v;
    }
    if total > 0 {
        let black = bgra.chunks_exact(4).filter(|p| p[0] == 0).count();
        if black > total * 70 / 100 {
            for p in bgra.chunks_exact_mut(4) {
                p[0] = 255 - p[0];
                p[1] = 255 - p[1];
                p[2] = 255 - p[2];
            }
        }
    }
}

pub fn recognize_region(bgra: &[u8], width: u32, height: u32) -> Result<String, String> {
    let mut results: Vec<String> = Vec::new();

    if let Ok(bitmap) = write_bitmap(bgra, width, height) {
        if let Ok(text) = ocr_bitmap(&bitmap) {
            results.push(text);
        }
    }

    let scale = pick_scale(width, height);
    if scale < 8 {
        let mut pixels = bgra.to_vec();
        to_grayscale(&mut pixels);
        otsu_threshold(&mut pixels);
        let (pixels, rw, rh) = upscale(&pixels, width, height, scale);
        if let Ok(bitmap) = write_bitmap(&pixels, rw, rh) {
            if let Ok(text) = ocr_bitmap(&bitmap) {
                results.push(text);
            }
        }
    } else {
        let mut p1 = bgra.to_vec();
        to_grayscale(&mut p1);
        let (p1, rw1, rh1) = upscale(&p1, width, height, scale);
        if let Ok(bitmap) = write_bitmap(&p1, rw1, rh1) {
            if let Ok(text) = ocr_bitmap(&bitmap) {
                results.push(text);
            }
        }
        let mut p2 = bgra.to_vec();
        to_grayscale(&mut p2);
        threshold_fixed(&mut p2);
        let (p2, rw2, rh2) = upscale(&p2, width, height, scale);
        if let Ok(bitmap) = write_bitmap(&p2, rw2, rh2) {
            if let Ok(text) = ocr_bitmap(&bitmap) {
                results.push(text);
            }
        }
    }

    if results.is_empty() {
        return Err("No text found".into());
    }

    results.sort_by(|a, b| {
        let a_norm = normalize_fullwidth(a);
        let b_norm = normalize_fullwidth(b);
        let a_alpha = a_norm.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        let b_alpha = b_norm.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        b_alpha.cmp(&a_alpha)
    });

    Ok(results[0].clone())
}
