//! Presentation-only Shell icons. Runs on an independent STA so Shell handlers
//! cannot block selection polling. Never launches executables or persists icons.
use base64::Engine;
use serde_json::{json, Value};
use std::{mem::size_of, path::Path};
use windows::{
    core::{Interface, PCWSTR},
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::*,
        System::Com::{CoTaskMemFree, IBindCtx},
        UI::Shell::*,
    },
};

/// Use the installed Apps folder for name-only presets (including Store apps).
/// Names select artwork only: they never grant capture permission or change the
/// existing executable identity rule. Ambiguous/missing matches use UI fallback.
unsafe fn preset_items() -> Vec<(String, IShellItem)> {
    let Ok(folder) =
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
    else {
        return vec![];
    };
    let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None::<&IBindCtx>, &BHID_EnumItems)
    else {
        return vec![];
    };
    let mut found = Vec::new();
    // Bound enumeration even if a broken Shell provider never reaches the end.
    for _ in 0..2048 {
        let mut next = [None];
        let mut fetched = 0;
        if items.Next(&mut next, Some(&mut fetched)).is_err() || fetched == 0 {
            break;
        }
        let Some(item) = next[0].take() else { break };
        let Ok(raw) = item.GetDisplayName(SIGDN_NORMALDISPLAY) else {
            continue;
        };
        let name = raw.to_string().unwrap_or_default().to_lowercase();
        CoTaskMemFree(Some(raw.0.cast()));
        let preset = match name.as_str() {
            "chatgpt" | "chatgpt desktop" => "chatgpt",
            "claude" | "claude desktop" => "claude",
            _ => continue,
        };
        found.push((preset.into(), item));
    }
    found
}

/// Own the bitmap and DC even on PNG/conversion errors. GetDIBits requires an
/// unselected bitmap; no Shell-owned handle or COM object escapes this worker.
struct IconBitmap {
    bitmap: HBITMAP,
    dc: HDC,
}
impl Drop for IconBitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Shell returns premultiplied BGRA. Export top-down straight-alpha RGBA so the
/// same icon blends correctly on both light and dark settings backgrounds.
unsafe fn png_icon(item: &IShellItem) -> Option<String> {
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let bitmap = factory
        .GetImage(SIZE { cx: 64, cy: 64 }, SIIGBF_ICONONLY)
        .ok()?;
    let owner = IconBitmap {
        bitmap,
        dc: CreateCompatibleDC(None),
    };
    if owner.dc.is_invalid() {
        return None;
    }
    let mut details = BITMAP::default();
    if GetObjectW(
        bitmap.into(),
        size_of::<BITMAP>() as i32,
        Some((&mut details as *mut BITMAP).cast()),
    ) == 0
    {
        return None;
    }
    let (width, height) = (details.bmWidth, details.bmHeight);
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    if GetDIBits(
        owner.dc,
        bitmap,
        0,
        height as u32,
        Some(rgba.as_mut_ptr().cast()),
        &mut info,
        DIB_RGB_COLORS,
    ) != height
    {
        return None;
    }
    // Older Shell icons can be RGB-only; their unused alpha byte is all zero.
    let opaque = rgba.chunks_exact(4).all(|pixel| pixel[3] == 0);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if opaque {
            pixel[3] = 255;
        }
        let alpha = pixel[3] as u32;
        if alpha > 0 && alpha < 255 {
            for color in &mut pixel[..3] {
                *color = ((*color as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().ok()?.write_image_data(&rgba).ok()?;
    }
    if bytes.len() > 65536 {
        return None;
    }
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Keep result positions aligned with the requested draft. Invalid paths,
/// removed executables and individual Shell failures return null, not a failed
/// settings page. A custom icon must come from a validated local .exe path.
pub unsafe fn read(request: &Value) -> Result<Value, String> {
    let apps = request["apps"].as_array().ok_or("无效的应用列表。")?;
    if apps.len() > 100 {
        return Err("应用数量过多。".into());
    }
    let presets = if apps.iter().any(Value::is_string) {
        preset_items()
    } else {
        vec![]
    };
    let icons: Vec<Option<String>> = apps
        .iter()
        .map(|app| {
            if let Some(preset) = app.as_str() {
                let mut matches = presets.iter().filter(|(name, _)| name == preset);
                let item = &matches.next()?.1;
                return if matches.next().is_none() {
                    png_icon(item)
                } else {
                    None
                };
            }
            if app["platform"] != "Windows" {
                return None;
            }
            let path = app["path"].as_str()?;
            let actual = super::application_descriptor(Path::new(path)).ok()?;
            if actual["path"].as_str()? != path {
                return None;
            }
            let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
            let item = SHCreateItemFromParsingName::<_, _, IShellItem>(
                PCWSTR(wide.as_ptr()),
                None::<&IBindCtx>,
            )
            .ok()?;
            png_icon(&item)
        })
        .collect();
    Ok(json!({"icons": icons}))
}
