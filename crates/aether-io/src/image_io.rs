//! Image import and export.

use aether_core::color::Rgba8;
use aether_core::{AetherError, Result};
use aether_raster::{Mask, Pixmap};
use image::{DynamicImage, ImageFormat as CrateFormat, RgbaImage};
use std::io::Cursor;
use std::path::Path;

/// Formats the application can read and write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// Lossless with alpha; the default for artwork.
    Png,
    /// Lossy, no alpha.
    Jpeg,
    /// Lossless or lossy with alpha.
    WebP,
    /// Lossless, wide tool support.
    Tiff,
    /// Uncompressed.
    Bmp,
    /// Palette-based, animation capable.
    Gif,
}

impl ImageFormat {
    /// Guess the format from a file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        Some(match ext.as_str() {
            "png" => ImageFormat::Png,
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "webp" => ImageFormat::WebP,
            "tif" | "tiff" => ImageFormat::Tiff,
            "bmp" => ImageFormat::Bmp,
            "gif" => ImageFormat::Gif,
            _ => return None,
        })
    }

    /// The canonical file extension.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::WebP => "webp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Gif => "gif",
        }
    }

    /// Whether the format stores an alpha channel.
    pub fn supports_alpha(self) -> bool {
        !matches!(self, ImageFormat::Jpeg | ImageFormat::Bmp)
    }

    fn to_crate_format(self) -> CrateFormat {
        match self {
            ImageFormat::Png => CrateFormat::Png,
            ImageFormat::Jpeg => CrateFormat::Jpeg,
            ImageFormat::WebP => CrateFormat::WebP,
            ImageFormat::Tiff => CrateFormat::Tiff,
            ImageFormat::Bmp => CrateFormat::Bmp,
            ImageFormat::Gif => CrateFormat::Gif,
        }
    }
}

/// Settings for one export.
#[derive(Clone, Debug)]
pub struct ExportSettings {
    /// Target format.
    pub format: ImageFormat,
    /// Scale factor applied before encoding.
    pub scale: f32,
    /// Colour composited behind the image when the format has no alpha.
    pub matte: Rgba8,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            scale: 1.0,
            matte: Rgba8::WHITE,
        }
    }
}

/// Decode an image file into a [`Pixmap`].
pub fn load_image(path: impl AsRef<Path>) -> Result<Pixmap> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    decode_image(&bytes)
}

/// Decode image bytes into a [`Pixmap`].
pub fn decode_image(bytes: &[u8]) -> Result<Pixmap> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| AetherError::UnsupportedFormat(format!("could not decode image: {e}")))?;
    Ok(from_dynamic(image))
}

fn from_dynamic(image: DynamicImage) -> Pixmap {
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    // `image` hands back tightly packed RGBA8, exactly the Pixmap layout.
    Pixmap::from_raw(w, h, rgba.into_raw()).unwrap_or_else(|_| Pixmap::new(w, h))
}

fn to_rgba_image(pixmap: &Pixmap) -> Option<RgbaImage> {
    RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.data().to_vec())
}

/// Write a [`Pixmap`] as a PNG.
pub fn save_png(pixmap: &Pixmap, path: impl AsRef<Path>) -> Result<()> {
    export_image(pixmap, path, &ExportSettings::default())
}

/// Encode a [`Pixmap`] into memory.
pub fn encode_image(pixmap: &Pixmap, settings: &ExportSettings) -> Result<Vec<u8>> {
    let scaled = if (settings.scale - 1.0).abs() > 1e-4 {
        let w = ((pixmap.width() as f32 * settings.scale).round() as u32).max(1);
        let h = ((pixmap.height() as f32 * settings.scale).round() as u32).max(1);
        pixmap.scaled(w, h)
    } else {
        pixmap.clone()
    };

    let flattened = if settings.format.supports_alpha() {
        scaled
    } else {
        aether_render::checker::over_color(&scaled, settings.matte)
    };

    let rgba = to_rgba_image(&flattened)
        .ok_or_else(|| AetherError::raster("pixel buffer could not be wrapped for encoding"))?;
    let dynamic = if settings.format.supports_alpha() {
        DynamicImage::ImageRgba8(rgba)
    } else {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
    };

    let mut buffer = Cursor::new(Vec::new());
    dynamic
        .write_to(&mut buffer, settings.format.to_crate_format())
        .map_err(|e| AetherError::UnsupportedFormat(format!("could not encode image: {e}")))?;
    Ok(buffer.into_inner())
}

/// Write a [`Pixmap`] to disk using `settings`.
pub fn export_image(pixmap: &Pixmap, path: impl AsRef<Path>, settings: &ExportSettings) -> Result<()> {
    let path = path.as_ref();
    if pixmap.is_empty() {
        return Err(AetherError::invalid("cannot export an empty image"));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let bytes = encode_image(pixmap, settings)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Encode a [`Mask`] as an 8-bit grayscale PNG.
pub fn encode_mask_png(mask: &Mask) -> Result<Vec<u8>> {
    let img = image::GrayImage::from_raw(mask.width(), mask.height(), mask.data().to_vec())
        .ok_or_else(|| AetherError::raster("mask buffer could not be wrapped for encoding"))?;
    let mut buffer = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(img)
        .write_to(&mut buffer, CrateFormat::Png)
        .map_err(|e| AetherError::serialization(format!("could not encode mask: {e}")))?;
    Ok(buffer.into_inner())
}

/// Decode a grayscale PNG back into a [`Mask`].
pub fn decode_mask_png(bytes: &[u8]) -> Result<Mask> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| AetherError::serialization(format!("could not decode mask: {e}")))?;
    let gray = image.to_luma8();
    let (w, h) = gray.dimensions();
    Mask::from_raw(w, h, gray.into_raw())
}

/// Encode a [`Pixmap`] as a PNG (used by the project container).
pub fn encode_pixmap_png(pixmap: &Pixmap) -> Result<Vec<u8>> {
    encode_image(pixmap, &ExportSettings::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::IRect;

    fn sample() -> Pixmap {
        let mut pm = Pixmap::new(8, 8);
        pm.fill_rect(IRect::new(0, 0, 4, 8), Rgba8::new(200, 100, 50, 255));
        pm.fill_rect(IRect::new(4, 0, 4, 8), Rgba8::new(0, 0, 0, 128));
        pm
    }

    #[test]
    fn png_round_trip_is_lossless() {
        let pm = sample();
        let bytes = encode_pixmap_png(&pm).expect("encode");
        let back = decode_image(&bytes).expect("decode");
        assert_eq!(back, pm);
    }

    #[test]
    fn format_detection_from_extension() {
        assert_eq!(
            ImageFormat::from_path(Path::new("a/b.PNG")),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::from_path(Path::new("a/b.jpeg")),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::from_path(Path::new("a/b.xcf")), None);
    }

    #[test]
    fn jpeg_export_flattens_alpha_onto_the_matte() {
        let pm = Pixmap::new(4, 4);
        let settings = ExportSettings {
            format: ImageFormat::Jpeg,
            matte: Rgba8::rgb(255, 0, 0),
            ..Default::default()
        };
        let bytes = encode_image(&pm, &settings).expect("encode");
        let back = decode_image(&bytes).expect("decode");
        assert_eq!(back.get(0, 0).a, 255, "jpeg has no alpha");
        assert!(
            back.get(0, 0).r > 200,
            "matte colour should show through: {:?}",
            back.get(0, 0)
        );
    }

    #[test]
    fn scaled_export_changes_dimensions() {
        let pm = sample();
        let settings = ExportSettings {
            scale: 2.0,
            ..Default::default()
        };
        let bytes = encode_image(&pm, &settings).expect("encode");
        let back = decode_image(&bytes).expect("decode");
        assert_eq!((back.width(), back.height()), (16, 16));
    }

    #[test]
    fn mask_round_trip() {
        let mut mask = Mask::new(8, 4);
        mask.fill_rect(IRect::new(1, 1, 3, 2), 200);
        let bytes = encode_mask_png(&mask).expect("encode");
        let back = decode_mask_png(&bytes).expect("decode");
        assert_eq!(back, mask);
    }

    #[test]
    fn exporting_an_empty_image_is_an_error() {
        let pm = Pixmap::new(0, 0);
        let err = export_image(&pm, "/tmp/aether-empty.png", &ExportSettings::default())
            .expect_err("empty export must fail");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn garbage_bytes_produce_a_clear_error() {
        let err = decode_image(b"not an image at all").expect_err("must fail");
        assert!(matches!(err, AetherError::UnsupportedFormat(_)), "got {err:?}");
    }

    #[test]
    fn webp_round_trip_keeps_alpha() {
        let pm = sample();
        let settings = ExportSettings {
            format: ImageFormat::WebP,
            ..Default::default()
        };
        let bytes = encode_image(&pm, &settings).expect("encode");
        let back = decode_image(&bytes).expect("decode");
        assert_eq!((back.width(), back.height()), (8, 8));
        assert!(
            back.get(6, 1).a < 255,
            "alpha should survive: {:?}",
            back.get(6, 1)
        );
    }
}
