use tauri::{image::Image, AppHandle, Theme};

#[cfg(not(target_os = "macos"))]
use tauri::Manager;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppTheme {
    Cave,
    Aurora,
    Ember,
    Daylight,
}

#[derive(Clone, Copy)]
struct IconPalette {
    background: [u8; 3],
    mark: [u8; 3],
}

impl AppTheme {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "cave" => Ok(Self::Cave),
            "aurora" => Ok(Self::Aurora),
            "ember" => Ok(Self::Ember),
            "daylight" => Ok(Self::Daylight),
            _ => Err("app_appearance_theme_invalid".to_string()),
        }
    }

    fn window_theme(self) -> Theme {
        match self {
            Self::Daylight => Theme::Light,
            Self::Cave | Self::Aurora | Self::Ember => Theme::Dark,
        }
    }

    fn palette(self) -> IconPalette {
        match self {
            Self::Cave => IconPalette {
                background: [0x11, 0x14, 0x17],
                mark: [0x4a, 0x9c, 0xff],
            },
            Self::Aurora => IconPalette {
                background: [0x07, 0x19, 0x22],
                mark: [0x5e, 0xea, 0xd4],
            },
            Self::Ember => IconPalette {
                background: [0x17, 0x11, 0x13],
                mark: [0xfb, 0xbf, 0x24],
            },
            Self::Daylight => IconPalette {
                background: [0xf4, 0xf7, 0xf4],
                mark: [0x04, 0x78, 0x57],
            },
        }
    }
}

pub(crate) fn sync(app: &AppHandle, theme: &str) -> Result<(), String> {
    let theme = AppTheme::parse(theme)?;
    let source = app
        .default_window_icon()
        .ok_or_else(|| "app_appearance_default_icon_missing".to_string())?;
    let icon = themed_icon(source, theme);

    app.set_theme(Some(theme.window_theme()));

    #[cfg(target_os = "macos")]
    set_macos_dock_icon(app, &icon)?;

    #[cfg(not(target_os = "macos"))]
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_icon(icon)
            .map_err(|error| format!("app_appearance_window_icon_failed:{error}"))?;
    }

    Ok(())
}

fn themed_icon(source: &Image<'_>, theme: AppTheme) -> Image<'static> {
    let palette = theme.palette();
    let mut rgba = source.rgba().to_vec();
    let width = source.width() as usize;
    let should_mask = source.width().min(source.height()) >= 16;

    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        if should_mask {
            let coverage = rounded_icon_coverage(
                source.width(),
                source.height(),
                (index % width) as u32,
                (index / width) as u32,
            );
            pixel[3] = (f32::from(pixel[3]) * coverage).round() as u8;
        }
        if pixel[3] == 0 {
            continue;
        }

        let source_rgb = [pixel[0], pixel[1], pixel[2]];
        let luminance = luminance(source_rgb);
        let target = if is_brand_mark(source_rgb) {
            shade_mark(palette.mark, luminance)
        } else {
            shade_background(palette.background, luminance)
        };
        pixel[..3].copy_from_slice(&target);
    }

    Image::new_owned(rgba, source.width(), source.height())
}

fn rounded_icon_coverage(width: u32, height: u32, x: u32, y: u32) -> f32 {
    let size = width.min(height) as f32;
    let inset = size * 0.028;
    let radius = size * 0.205;
    let half_width = width as f32 / 2.0 - inset;
    let half_height = height as f32 / 2.0 - inset;
    let point_x = x as f32 + 0.5 - width as f32 / 2.0;
    let point_y = y as f32 + 0.5 - height as f32 / 2.0;
    let corner_x = point_x.abs() - (half_width - radius);
    let corner_y = point_y.abs() - (half_height - radius);
    let outside = corner_x.max(0.0).hypot(corner_y.max(0.0));
    let inside = corner_x.max(corner_y).min(0.0);
    let signed_distance = outside + inside - radius;
    (0.5 - signed_distance).clamp(0.0, 1.0)
}

fn is_brand_mark([red, green, blue]: [u8; 3]) -> bool {
    blue >= 96 && blue.saturating_sub(red) >= 24 && blue.saturating_sub(green) >= 4
}

fn luminance([red, green, blue]: [u8; 3]) -> u8 {
    ((u16::from(red) * 54 + u16::from(green) * 183 + u16::from(blue) * 19) / 256) as u8
}

fn shade_mark(mark: [u8; 3], source_luminance: u8) -> [u8; 3] {
    let factor = 0.72 + f32::from(source_luminance) / 255.0 * 0.62;
    mark.map(|channel| (f32::from(channel) * factor).round().clamp(0.0, 255.0) as u8)
}

fn shade_background(background: [u8; 3], source_luminance: u8) -> [u8; 3] {
    let adjustment = (f32::from(source_luminance) - 22.0) * 0.48;
    background.map(|channel| (f32::from(channel) + adjustment).round().clamp(0.0, 255.0) as u8)
}

#[cfg(target_os = "macos")]
fn set_macos_dock_icon(app: &AppHandle, icon: &Image<'_>) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let png = encode_png(icon)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = sender.send(apply_macos_dock_icon(&png));
    })
    .map_err(|error| format!("app_appearance_main_thread_failed:{error}"))?;
    receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("app_appearance_main_thread_timeout:{error}"))?
}

#[cfg(target_os = "macos")]
fn encode_png(icon: &Image<'_>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, icon.width(), icon.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("app_appearance_png_header_failed:{error}"))?;
        writer
            .write_image_data(icon.rgba())
            .map_err(|error| format!("app_appearance_png_write_failed:{error}"))?;
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
fn apply_macos_dock_icon(png: &[u8]) -> Result<(), String> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let marker =
        MainThreadMarker::new().ok_or_else(|| "app_appearance_not_on_main_thread".to_string())?;
    let data = NSData::with_bytes(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "app_appearance_nsimage_decode_failed".to_string())?;
    let application = NSApplication::sharedApplication(marker);
    unsafe { application.setApplicationIconImage(Some(&image)) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_preferences_that_have_not_been_resolved() {
        assert_eq!(
            AppTheme::parse("system"),
            Err("app_appearance_theme_invalid".to_string())
        );
    }

    #[test]
    fn maps_light_and_dark_native_appearance() {
        assert_eq!(AppTheme::Daylight.window_theme(), Theme::Light);
        assert_eq!(AppTheme::Cave.window_theme(), Theme::Dark);
        assert_eq!(AppTheme::Aurora.window_theme(), Theme::Dark);
        assert_eq!(AppTheme::Ember.window_theme(), Theme::Dark);
    }

    #[test]
    fn recolors_the_mark_and_background_for_each_resolved_theme() {
        let source = Image::new(&[30, 42, 210, 255, 20, 22, 24, 255], 2, 1);
        let ember = themed_icon(&source, AppTheme::Ember);
        let daylight = themed_icon(&source, AppTheme::Daylight);

        assert!(ember.rgba()[0] > ember.rgba()[1]);
        assert!(ember.rgba()[1] > ember.rgba()[2]);
        assert!(daylight.rgba()[1] > daylight.rgba()[0]);
        assert!(daylight.rgba()[4] > ember.rgba()[4]);
        assert_eq!(ember.width(), source.width());
        assert_eq!(ember.height(), source.height());
    }

    #[test]
    fn preserves_transparent_pixels() {
        let source = Image::new(&[12, 34, 56, 0], 1, 1);
        let themed = themed_icon(&source, AppTheme::Daylight);
        assert_eq!(themed.rgba(), source.rgba());
    }

    #[test]
    fn masks_the_opaque_source_square_to_the_app_icon_shape() {
        let source = Image::new_owned(vec![255; 64 * 64 * 4], 64, 64);
        let themed = themed_icon(&source, AppTheme::Daylight);
        let alpha = |x: usize, y: usize| themed.rgba()[(y * 64 + x) * 4 + 3];

        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(63, 0), 0);
        assert_eq!(alpha(0, 63), 0);
        assert_eq!(alpha(63, 63), 0);
        assert_eq!(alpha(32, 32), 255);
    }
}
