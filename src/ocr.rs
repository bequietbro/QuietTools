use windows::core::Interface;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Win32::System::WinRT::IMemoryBufferByteAccess;
use windows_future::AsyncStatus;

fn has_non_ascii(s: &str) -> bool {
    s.chars().any(|c| c > '\u{007F}')
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

pub fn recognize_region(bgra: &[u8], width: u32, height: u32) -> Result<String, String> {
    let scale = if height < 40 || width < 80 { 2 } else { 1 };
    let (pixels, rw, rh) = if scale > 1 {
        upscale(bgra, width, height, scale)
    } else {
        (bgra.to_vec(), width, height)
    };

    let bitmap = write_bitmap(&pixels, rw, rh)?;

    let langs = OcrEngine::AvailableRecognizerLanguages()
        .map_err(|_| "OCR: install a language pack")?;

    let lang_count = langs.Size().unwrap_or(0);
    if lang_count == 0 {
        return Err("OCR: install a language pack".into());
    }

    let mut fallback: Option<String> = None;

    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        if let Ok(text) = try_engine(&bitmap, &engine) {
            let t = text.trim().to_string();
            if !t.is_empty() {
                if has_non_ascii(&t) {
                    return Ok(t);
                }
                fallback = Some(t);
            }
        }
    }

    for lang in &langs {
        if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
            if let Ok(text) = try_engine(&bitmap, &engine) {
                let t = text.trim().to_string();
                if !t.is_empty() {
                    if has_non_ascii(&t) {
                        return Ok(t);
                    }
                    if fallback.is_none() {
                        fallback = Some(t);
                    }
                }
            }
        }
    }

    if let Some(text) = fallback {
        return Ok(text);
    }

    Err("No text found".into())
}
