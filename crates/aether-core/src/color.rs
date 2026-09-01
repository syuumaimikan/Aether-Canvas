//! Colour types and conversions.
//!
//! The painting pipeline works in **straight (non-premultiplied) alpha**:
//! [`Rgba`] holds `f32` channels in `0..=1` for maths, [`Rgba8`] is the 8-bit
//! storage format used by pixel buffers. Keeping alpha straight makes the W3C
//! blend formulas in [`crate::blend`] map directly onto the code, and avoids
//! the rounding drift premultiplied 8-bit storage suffers from when a stroke is
//! repeatedly re-composited.
//!
//! Channel values are treated as sRGB-encoded. Conversion helpers to and from
//! linear light are provided for effects that need physically correct maths.

use crate::math::clampf;
use serde::{Deserialize, Serialize};

/// Floating point RGBA colour with straight alpha, channels in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel (1 = opaque).
    pub a: f32,
}

impl Default for Rgba {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

impl Rgba {
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    /// Opaque black.
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    /// Opaque white.
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    /// Construct from components (values are clamped).
    #[inline]
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: clampf(r, 0.0, 1.0),
            g: clampf(g, 0.0, 1.0),
            b: clampf(b, 0.0, 1.0),
            a: clampf(a, 0.0, 1.0),
        }
    }

    /// Opaque colour from RGB components.
    #[inline]
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    /// Opaque grey.
    #[inline]
    pub fn gray(v: f32) -> Self {
        Self::new(v, v, v, 1.0)
    }

    /// The same colour with a different alpha.
    #[inline]
    pub fn with_alpha(self, a: f32) -> Self {
        Self {
            a: clampf(a, 0.0, 1.0),
            ..self
        }
    }

    /// Perceptual luminance (Rec. 709 weights).
    #[inline]
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Per-channel linear interpolation.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }

    /// Convert to 8-bit storage.
    #[inline]
    pub fn to_rgba8(self) -> Rgba8 {
        Rgba8 {
            r: to_u8(self.r),
            g: to_u8(self.g),
            b: to_u8(self.b),
            a: to_u8(self.a),
        }
    }

    /// Decode this sRGB colour to linear light (alpha untouched).
    pub fn to_linear(self) -> Self {
        Self {
            r: srgb_to_linear(self.r),
            g: srgb_to_linear(self.g),
            b: srgb_to_linear(self.b),
            a: self.a,
        }
    }

    /// Encode this linear colour back to sRGB (alpha untouched).
    pub fn to_srgb(self) -> Self {
        Self {
            r: linear_to_srgb(self.r),
            g: linear_to_srgb(self.g),
            b: linear_to_srgb(self.b),
            a: self.a,
        }
    }

    /// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` (the leading `#` is optional).
    pub fn from_hex(text: &str) -> Option<Self> {
        Some(Rgba8::from_hex(text)?.to_rgba())
    }

    /// Format as `#rrggbb`, or `#rrggbbaa` when not fully opaque.
    pub fn to_hex(self) -> String {
        self.to_rgba8().to_hex()
    }
}

/// 8-bit RGBA colour with straight alpha; the in-memory pixel format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba8 {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

impl Rgba8 {
    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    /// Opaque black.
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    /// Opaque white.
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    /// Construct from components.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Opaque colour from RGB components.
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// As a `[r, g, b, a]` array.
    #[inline]
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// From a `[r, g, b, a]` array.
    #[inline]
    pub const fn from_array(v: [u8; 4]) -> Self {
        Self {
            r: v[0],
            g: v[1],
            b: v[2],
            a: v[3],
        }
    }

    /// Convert to floating point.
    #[inline]
    pub fn to_rgba(self) -> Rgba {
        Rgba {
            r: self.r as f32 / 255.0,
            g: self.g as f32 / 255.0,
            b: self.b as f32 / 255.0,
            a: self.a as f32 / 255.0,
        }
    }

    /// Parse `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(text: &str) -> Option<Self> {
        let t = text.trim().trim_start_matches('#');
        let nib = |i: usize| -> Option<u8> {
            t.as_bytes()
                .get(i)
                .and_then(|c| (*c as char).to_digit(16))
                .map(|d| d as u8)
        };
        match t.len() {
            3 | 4 => {
                let r = nib(0)?;
                let g = nib(1)?;
                let b = nib(2)?;
                let a = if t.len() == 4 { nib(3)? } else { 0xF };
                Some(Self::new(r * 17, g * 17, b * 17, a * 17))
            }
            6 | 8 => {
                let byte = |i: usize| -> Option<u8> { Some(nib(i * 2)? * 16 + nib(i * 2 + 1)?) };
                let a = if t.len() == 8 { byte(3)? } else { 255 };
                Some(Self::new(byte(0)?, byte(1)?, byte(2)?, a))
            }
            _ => None,
        }
    }

    /// Format as `#rrggbb`, or `#rrggbbaa` when not fully opaque.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

/// Hue / saturation / value colour, the model behind the colour wheel.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hsv {
    /// Hue in degrees, `0..360`.
    pub h: f32,
    /// Saturation, `0..=1`.
    pub s: f32,
    /// Value, `0..=1`.
    pub v: f32,
    /// Alpha, `0..=1`.
    pub a: f32,
}

impl Hsv {
    /// Construct from components.
    pub fn new(h: f32, s: f32, v: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: clampf(s, 0.0, 1.0),
            v: clampf(v, 0.0, 1.0),
            a: clampf(a, 0.0, 1.0),
        }
    }

    /// Convert from RGBA.
    pub fn from_rgba(c: Rgba) -> Self {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let delta = max - min;
        let h = if delta <= f32::EPSILON {
            0.0
        } else if max == c.r {
            60.0 * (((c.g - c.b) / delta) % 6.0)
        } else if max == c.g {
            60.0 * ((c.b - c.r) / delta + 2.0)
        } else {
            60.0 * ((c.r - c.g) / delta + 4.0)
        };
        let s = if max <= f32::EPSILON { 0.0 } else { delta / max };
        Self::new(h, s, max, c.a)
    }

    /// Convert to RGBA.
    pub fn to_rgba(self) -> Rgba {
        let c = self.v * self.s;
        let hp = self.h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r1, g1, b1) = match hp as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        Rgba::new(r1 + m, g1 + m, b1 + m, self.a)
    }

    /// Rotate the hue by `degrees`, useful for colour harmonies.
    pub fn rotated(self, degrees: f32) -> Self {
        Self::new(self.h + degrees, self.s, self.v, self.a)
    }
}

/// Hue / saturation / lightness colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hsl {
    /// Hue in degrees, `0..360`.
    pub h: f32,
    /// Saturation, `0..=1`.
    pub s: f32,
    /// Lightness, `0..=1`.
    pub l: f32,
    /// Alpha, `0..=1`.
    pub a: f32,
}

impl Hsl {
    /// Construct from components.
    pub fn new(h: f32, s: f32, l: f32, a: f32) -> Self {
        Self {
            h: h.rem_euclid(360.0),
            s: clampf(s, 0.0, 1.0),
            l: clampf(l, 0.0, 1.0),
            a: clampf(a, 0.0, 1.0),
        }
    }

    /// Convert from RGBA.
    pub fn from_rgba(c: Rgba) -> Self {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let l = (max + min) * 0.5;
        let delta = max - min;
        if delta <= f32::EPSILON {
            return Self::new(0.0, 0.0, l, c.a);
        }
        let s = delta / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON);
        let h = if max == c.r {
            60.0 * (((c.g - c.b) / delta) % 6.0)
        } else if max == c.g {
            60.0 * ((c.b - c.r) / delta + 2.0)
        } else {
            60.0 * ((c.r - c.g) / delta + 4.0)
        };
        Self::new(h, s, l, c.a)
    }

    /// Convert to RGBA.
    pub fn to_rgba(self) -> Rgba {
        let c = (1.0 - (2.0 * self.l - 1.0).abs()) * self.s;
        let hp = self.h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r1, g1, b1) = match hp as i32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.l - c * 0.5;
        Rgba::new(r1 + m, g1 + m, b1 + m, self.a)
    }
}

/// Colour models a document can be authored in.
///
/// Phase 1 stores pixels as 8-bit sRGB RGBA; this enum records the *authoring*
/// intent so exporters and the colour picker can present the right controls,
/// and so later phases can add real 16-bit and CMYK pipelines without changing
/// the project schema.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorModel {
    /// Standard 8-bit sRGB with alpha.
    #[default]
    Rgba8,
    /// Single channel luminance with alpha.
    Grayscale,
    /// Indexed palette colour (pixel-art workflows).
    Indexed,
}

/// Decode one sRGB-encoded channel to linear light.
#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode one linear-light channel as sRGB.
#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (clampf(v, 0.0, 1.0) * 255.0 + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 2e-3
    }

    #[test]
    fn rgba8_roundtrip() {
        for v in [0u8, 1, 37, 128, 254, 255] {
            let c = Rgba8::new(v, v, v, v);
            assert_eq!(c.to_rgba().to_rgba8(), c);
        }
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(Rgba8::from_hex("#ff8800"), Some(Rgba8::rgb(255, 136, 0)));
        assert_eq!(Rgba8::from_hex("f80"), Some(Rgba8::rgb(255, 136, 0)));
        assert_eq!(Rgba8::from_hex("#ff880080").map(|c| c.a), Some(128));
        assert_eq!(Rgba8::from_hex("zzz"), None);
        assert_eq!(Rgba8::from_hex("#12345"), None);
        assert_eq!(Rgba8::rgb(255, 136, 0).to_hex(), "#ff8800");
    }

    #[test]
    fn hsv_roundtrip() {
        for c in [
            Rgba::rgb(1.0, 0.0, 0.0),
            Rgba::rgb(0.2, 0.7, 0.35),
            Rgba::gray(0.5),
            Rgba::rgb(0.0, 0.0, 1.0),
        ] {
            let back = Hsv::from_rgba(c).to_rgba();
            assert!(
                approx(back.r, c.r) && approx(back.g, c.g) && approx(back.b, c.b),
                "{c:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn hsl_roundtrip() {
        for c in [
            Rgba::rgb(1.0, 0.0, 0.0),
            Rgba::rgb(0.2, 0.7, 0.35),
            Rgba::gray(0.25),
        ] {
            let back = Hsl::from_rgba(c).to_rgba();
            assert!(
                approx(back.r, c.r) && approx(back.g, c.g) && approx(back.b, c.b),
                "{c:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn srgb_linear_roundtrip() {
        for v in [0.0, 0.01, 0.25, 0.5, 1.0] {
            assert!(approx(linear_to_srgb(srgb_to_linear(v)), v));
        }
    }
}
