use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[cfg(any(windows, target_os = "macos"))]
use base64::{engine::general_purpose::STANDARD, Engine as _};

static ICON_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

pub fn icon_data_url(exe: &str) -> Result<Option<String>, String> {
    let exe = exe.trim();
    if exe.is_empty() {
        return Ok(None);
    }

    let key = exe.to_ascii_lowercase();
    let cache = ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(icon) = cache
        .lock()
        .map_err(|_| "process_icon_cache_poisoned".to_string())?
        .get(&key)
        .cloned()
    {
        return Ok(icon);
    }

    let icon = load_icon_data_url(exe);
    let mut cache = cache
        .lock()
        .map_err(|_| "process_icon_cache_poisoned".to_string())?;
    if cache.len() >= 256 {
        if let Some(oldest) = cache.keys().next().cloned() {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, icon.clone());
    Ok(icon)
}

#[cfg(windows)]
fn load_icon_data_url(exe: &str) -> Option<String> {
    let icon = extract_icon(exe)?;
    let bytes = icon_to_ico_bytes(icon.raw())?;
    Some(format!(
        "data:image/x-icon;base64,{}",
        STANDARD.encode(&bytes)
    ))
}

#[cfg(target_os = "macos")]
fn load_icon_data_url(exe: &str) -> Option<String> {
    let icon_path = find_macos_icns(std::path::Path::new(exe))?;
    let metadata = std::fs::metadata(&icon_path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 32 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(icon_path).ok()?;
    let png = decode_icns_to_png(&bytes)?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(&png)))
}

#[cfg(target_os = "macos")]
fn decode_icns_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let family = icns::IconFamily::read(std::io::Cursor::new(bytes)).ok()?;
    let mut icon_types = family.available_icons();
    icon_types.sort_by_key(|icon_type| {
        (
            icon_type.pixel_width().abs_diff(128),
            std::cmp::Reverse(icon_type.pixel_width()),
        )
    });

    icon_types.into_iter().find_map(|icon_type| {
        let image = family.get_icon_with_type(icon_type).ok()?;
        let mut png = Vec::new();
        image.write_png(&mut png).ok()?;
        Some(png)
    })
}

#[cfg(target_os = "macos")]
fn find_macos_icns(executable: &std::path::Path) -> Option<std::path::PathBuf> {
    if !executable.is_absolute() {
        return None;
    }
    // Helper apps nested inside an app bundle usually ship no icon; fall back to
    // the enclosing bundle so helpers share their app's icon.
    let bundles = executable
        .ancestors()
        .filter(|candidate| {
            candidate
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .collect::<Vec<_>>();
    let outermost = bundles.len().saturating_sub(1);
    // A nested helper only counts when it declares or names its own icon; any other
    // .icns inside it is usually a document icon, so fall through to the app.
    bundles
        .into_iter()
        .enumerate()
        .find_map(|(index, bundle)| bundle_icns(bundle, index == outermost))
}

#[cfg(target_os = "macos")]
fn declared_icon_file(bundle: &std::path::Path) -> Option<String> {
    let info = plist::Value::from_file(bundle.join("Contents").join("Info.plist")).ok()?;
    let name = info
        .as_dictionary()?
        .get("CFBundleIconFile")?
        .as_string()?
        .trim()
        .to_ascii_lowercase();
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(if name.ends_with(".icns") {
        name
    } else {
        format!("{name}.icns")
    })
}

#[cfg(target_os = "macos")]
fn bundle_icns(bundle: &std::path::Path, allow_any: bool) -> Option<std::path::PathBuf> {
    let canonical_bundle = bundle.canonicalize().ok()?;
    let declared = declared_icon_file(bundle);
    let resources = bundle.join("Contents").join("Resources");
    let bundle_stem = bundle
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut icons = std::fs::read_dir(resources)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("icns"))
        })
        .collect::<Vec<_>>();
    icons.sort_by_key(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let preferred_bundle_name = format!("{bundle_stem}.icns");
        match name.as_str() {
            _ if declared.as_deref() == Some(name.as_str()) => (0, name),
            "appicon.icns" => (1, name),
            _ if name == preferred_bundle_name => (2, name),
            "icon.icns" => (3, name),
            _ => (4, name),
        }
    });
    let named = |path: &std::path::PathBuf| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        declared.as_deref() == Some(name.as_str())
            || name == "appicon.icns"
            || name == format!("{bundle_stem}.icns")
            || name == "icon.icns"
    };
    icons.retain(|path| allow_any || named(path));
    icons.into_iter().find_map(|path| {
        let canonical = path.canonicalize().ok()?;
        canonical
            .starts_with(&canonical_bundle)
            .then_some(canonical)
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn load_icon_data_url(_exe: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn extract_icon(exe: &str) -> Option<IconHandle> {
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::Shell::ExtractIconExW;
    use windows_sys::Win32::UI::WindowsAndMessaging::HICON;

    let wide = exe
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut large: HICON = null_mut();
    let mut small: HICON = null_mut();
    let count = unsafe { ExtractIconExW(wide.as_ptr(), 0, &mut large, &mut small, 1) };
    if count == 0 {
        return None;
    }

    let selected = if !large.is_null() { large } else { small };
    let unused = if selected == large { small } else { large };
    if !unused.is_null() {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(unused);
        }
    }

    (!selected.is_null()).then_some(IconHandle(selected))
}

#[cfg(windows)]
struct IconHandle(windows_sys::Win32::UI::WindowsAndMessaging::HICON);

#[cfg(windows)]
impl IconHandle {
    fn raw(&self) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
        self.0
    }
}

#[cfg(windows)]
impl Drop for IconHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn icon_to_ico_bytes(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use std::mem::size_of;
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(icon, &mut info) } == 0 {
        return None;
    }

    let color_bitmap = info.hbmColor;
    let mask_bitmap = info.hbmMask;
    let result = if color_bitmap.is_null() {
        None
    } else {
        let mut bitmap = BITMAP::default();
        let got_object = unsafe {
            GetObjectW(
                color_bitmap.cast(),
                size_of::<BITMAP>() as i32,
                (&mut bitmap as *mut BITMAP).cast(),
            )
        };
        if got_object == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
            None
        } else {
            let width = bitmap.bmWidth as u32;
            let height = bitmap.bmHeight as u32;
            let mut bitmap_info = BITMAPINFO::default();
            bitmap_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bitmap_info.bmiHeader.biWidth = width as i32;
            bitmap_info.bmiHeader.biHeight = height as i32;
            bitmap_info.bmiHeader.biPlanes = 1;
            bitmap_info.bmiHeader.biBitCount = 32;
            bitmap_info.bmiHeader.biCompression = BI_RGB;
            bitmap_info.bmiHeader.biSizeImage = width.saturating_mul(height).saturating_mul(4);

            let hdc = unsafe { CreateCompatibleDC(null_mut()) };
            if hdc.is_null() {
                None
            } else {
                let mut pixels = vec![0_u8; bitmap_info.bmiHeader.biSizeImage as usize];
                let got_bits = unsafe {
                    GetDIBits(
                        hdc,
                        color_bitmap,
                        0,
                        height,
                        pixels.as_mut_ptr().cast(),
                        &mut bitmap_info,
                        DIB_RGB_COLORS,
                    )
                };
                unsafe {
                    DeleteDC(hdc);
                }

                if got_bits == 0 {
                    None
                } else {
                    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
                        for pixel in pixels.chunks_exact_mut(4) {
                            pixel[3] = 255;
                        }
                    }
                    Some(ico_bytes(width, height, &pixels))
                }
            }
        }
    };

    if !color_bitmap.is_null() {
        unsafe {
            DeleteObject(color_bitmap.cast());
        }
    }
    if !mask_bitmap.is_null() {
        unsafe {
            DeleteObject(mask_bitmap.cast());
        }
    }

    result
}

#[cfg(windows)]
fn ico_bytes(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mask_stride = width.div_ceil(32) * 4;
    let mask_size = mask_stride * height;
    let image_size = 40 + pixels.len() as u32 + mask_size;
    let mut bytes = Vec::with_capacity(22 + image_size as usize);

    write_u16(&mut bytes, 0);
    write_u16(&mut bytes, 1);
    write_u16(&mut bytes, 1);
    bytes.push(icon_dimension(width));
    bytes.push(icon_dimension(height));
    bytes.push(0);
    bytes.push(0);
    write_u16(&mut bytes, 1);
    write_u16(&mut bytes, 32);
    write_u32(&mut bytes, image_size);
    write_u32(&mut bytes, 22);

    write_u32(&mut bytes, 40);
    write_i32(&mut bytes, width as i32);
    write_i32(&mut bytes, height.saturating_mul(2) as i32);
    write_u16(&mut bytes, 1);
    write_u16(&mut bytes, 32);
    write_u32(&mut bytes, 0);
    write_u32(&mut bytes, pixels.len() as u32);
    write_i32(&mut bytes, 0);
    write_i32(&mut bytes, 0);
    write_u32(&mut bytes, 0);
    write_u32(&mut bytes, 0);
    bytes.extend_from_slice(pixels);
    bytes.resize(bytes.len() + mask_size as usize, 0);
    bytes
}

#[cfg(windows)]
fn icon_dimension(value: u32) -> u8 {
    if value >= 256 {
        0
    } else {
        value as u8
    }
}

#[cfg(windows)]
fn write_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(windows)]
fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(windows)]
fn write_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_exe_has_no_icon() {
        assert_eq!(icon_data_url("").unwrap(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_bundle_icns_is_returned_as_png_data_url() {
        let mut family = icns::IconFamily::new();
        let mut image = icns::Image::new(icns::PixelFormat::RGBA, 128, 128);
        for pixel in image.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0x72, 0xf1, 0xb8, 0xff]);
        }
        family.add_icon(&image).expect("ICNS icon encodes");
        let mut icns_bytes = Vec::new();
        family.write(&mut icns_bytes).expect("ICNS family writes");

        let root = std::env::temp_dir().join(format!(
            "batcave-macos-icon-{}-{}",
            std::process::id(),
            crate::telemetry::now_ms()
        ));
        let executable = root
            .join("Example.app")
            .join("Contents")
            .join("MacOS")
            .join("Example");
        let icon = root
            .join("Example.app")
            .join("Contents")
            .join("Resources")
            .join("AppIcon.icns");
        std::fs::create_dir_all(executable.parent().unwrap()).expect("executable directory");
        std::fs::create_dir_all(icon.parent().unwrap()).expect("icon directory");
        std::fs::write(&executable, b"binary").expect("executable fixture");
        std::fs::write(&icon, icns_bytes).expect("icon fixture");

        let data_url = load_icon_data_url(&executable.to_string_lossy()).expect("icon data URL");
        assert!(data_url.starts_with("data:image/png;base64,iVBORw0KGgo"));

        std::fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn nested_helper_without_icon_uses_the_enclosing_app_icon() {
        let root = std::env::temp_dir().join(format!(
            "batcave-macos-helper-icon-{}-{}",
            std::process::id(),
            crate::telemetry::now_ms()
        ));
        let app = root.join("Suite.app");
        let helper = app
            .join("Contents")
            .join("Frameworks")
            .join("Suite Helper.app")
            .join("Contents")
            .join("MacOS")
            .join("Suite Helper");
        let icon = app.join("Contents").join("Resources").join("Suite.icns");
        std::fs::create_dir_all(helper.parent().unwrap()).expect("helper directory");
        std::fs::create_dir_all(icon.parent().unwrap()).expect("icon directory");
        std::fs::write(&helper, b"binary").expect("helper fixture");
        std::fs::write(&icon, b"icns").expect("icon fixture");

        let found = find_macos_icns(&helper).expect("enclosing app icon is found");
        assert_eq!(found, icon.canonicalize().unwrap());

        std::fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn declared_bundle_icon_wins_over_document_icons() {
        let root = std::env::temp_dir().join(format!(
            "batcave-macos-declared-icon-{}-{}",
            std::process::id(),
            crate::telemetry::now_ms()
        ));
        let app = root.join("Visual Studio Code.app");
        let executable = app.join("Contents").join("MacOS").join("Electron");
        let resources = app.join("Contents").join("Resources");
        std::fs::create_dir_all(executable.parent().unwrap()).expect("executable directory");
        std::fs::create_dir_all(&resources).expect("resources directory");
        std::fs::write(&executable, b"binary").expect("executable fixture");
        for name in ["bat.icns", "Code.icns", "config.icns"] {
            std::fs::write(resources.join(name), b"icns").expect("icon fixture");
        }
        std::fs::write(
            app.join("Contents").join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIconFile</key><string>Code.icns</string></dict></plist>"#,
        )
        .expect("plist fixture");

        let found = find_macos_icns(&executable).expect("declared icon is found");
        assert_eq!(found, resources.join("Code.icns").canonicalize().unwrap());

        std::fs::remove_dir_all(root).expect("fixture cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn explorer_icon_is_data_url() {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let exe = std::path::Path::new(&windir).join("explorer.exe");
        assert!(exe.exists(), "explorer.exe exists");
        let icon = icon_data_url(&exe.to_string_lossy()).expect("icon lookup succeeds");
        assert!(icon
            .expect("explorer.exe exposes an icon")
            .starts_with("data:image/x-icon;base64,"));
    }
}
