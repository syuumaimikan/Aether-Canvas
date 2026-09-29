//! Drawing a Live2D model layer.
//!
//! Draws the model's visible drawables in render order, the way Cubism's
//! renderers do: each is a textured mesh with its opacity and multiply and
//! screen tints; clipped drawables show only where the union of their mask
//! drawables' texture alpha covers (or only outside it, for inverted masks);
//! additive and multiplicative drawables blend with what the model has drawn
//! below them; one-sided drawables drop triangles that face away.

use aether_core::blend::BlendMode;
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Vec2};
use aether_document::rig::cubism::{CubismBlend, CubismPose, CubismRig, CubismStatic};
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::mesh::{draw_textured_mesh, MeshDrawOptions};
use aether_raster::transform::Interpolation;
use aether_raster::{Mask, Pixmap};

/// A Cubism 5.3 colour blend type as the nearest blend mode.
pub fn blend_mode(blend: CubismBlend) -> BlendMode {
    match blend {
        CubismBlend::Normal => BlendMode::Normal,
        CubismBlend::Add => BlendMode::Add,
        CubismBlend::Multiply => BlendMode::Multiply,
        CubismBlend::Other(code) => match code {
            4 => BlendMode::Add,
            5 => BlendMode::Darken,
            7 => BlendMode::ColorBurn,
            8 => BlendMode::LinearBurn,
            9 => BlendMode::Lighten,
            10 => BlendMode::Screen,
            11 => BlendMode::ColorDodge,
            12 => BlendMode::Overlay,
            13 => BlendMode::SoftLight,
            14 => BlendMode::HardLight,
            15 => BlendMode::LinearLight,
            16 => BlendMode::Hue,
            17 => BlendMode::Color,
            _ => BlendMode::Normal,
        },
    }
}

/// Texture coordinates in the page's pixels.
fn texel_coords(s: &CubismStatic, texture: &Pixmap) -> Vec<Vec2> {
    let (w, h) = (texture.width() as f32, texture.height() as f32);
    s.uvs.iter().map(|uv| Vec2::new(uv.x * w, uv.y * h)).collect()
}

/// Triangles that face the viewer, for one-sided drawables. Cubism's front
/// faces wind counter-clockwise with y up, so their cross product is
/// negative in document space (y down).
fn facing(s: &CubismStatic, positions: &[Vec2]) -> Vec<[u32; 3]> {
    if s.double_sided {
        return s.triangles.clone();
    }
    s.triangles
        .iter()
        .copied()
        .filter(|t| {
            let (Some(a), Some(b), Some(c)) = (
                positions.get(t[0] as usize),
                positions.get(t[1] as usize),
                positions.get(t[2] as usize),
            ) else {
                return false;
            };
            (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x) <= 0.0
        })
        .collect()
}

fn bounds(positions: &[Vec2]) -> IRect {
    aether_document::rig::mesh::bounds_of(positions)
        .to_irect_outer()
        .expanded(1)
}

/// Draw the model on `layer`'s pixels (`width × height`, document space),
/// touching only `region`.
pub fn render(
    model: &CubismRig,
    textures: &[Pixmap],
    pose: &CubismPose,
    width: u32,
    height: u32,
    region: IRect,
    interpolation: Interpolation,
) -> Pixmap {
    let statics = model.statics();
    let mut out = Pixmap::new(width, height);
    let region = region.intersect(&out.bounds());
    if region.is_empty() {
        return out;
    }
    let mut scratch: Option<Pixmap> = None;
    let mut coverage: Option<(Pixmap, Mask)> = None;

    for &i in &pose.order {
        let (Some(s), Some(d)) = (statics.get(i), pose.drawables.get(i)) else {
            continue;
        };
        let Some(texture) = textures.get(s.texture) else {
            continue;
        };
        let area = bounds(&d.positions).intersect(&region);
        if area.is_empty() {
            continue;
        }
        let uvs = texel_coords(s, texture);
        let triangles = facing(s, &d.positions);
        let opts = MeshDrawOptions {
            region: area,
            opacity: d.opacity,
            multiply: d.multiply,
            screen: d.screen,
            interpolation,
        };
        if s.masks.is_empty() && s.blend == CubismBlend::Normal {
            draw_textured_mesh(&mut out, texture, &d.positions, &uvs, &triangles, &opts);
            continue;
        }

        let layer = scratch.get_or_insert_with(|| Pixmap::new(width, height));
        layer.fill_rect(area, Rgba8::TRANSPARENT);
        draw_textured_mesh(layer, texture, &d.positions, &uvs, &triangles, &opts);

        let mask = if s.masks.is_empty() {
            None
        } else {
            let (paint, mask) =
                coverage.get_or_insert_with(|| (Pixmap::new(width, height), Mask::new(width, height)));
            // The masks' texture alpha, unioned (source-over of alpha is
            // exactly 1 − Π(1 − a)), whatever their own opacity.
            paint.fill_rect(area, Rgba8::TRANSPARENT);
            for &m in &s.masks {
                let (Some(ms), Some(md)) = (statics.get(m), pose.drawables.get(m)) else {
                    continue;
                };
                let Some(mt) = textures.get(ms.texture) else {
                    continue;
                };
                let muv = texel_coords(ms, mt);
                let opts = MeshDrawOptions {
                    interpolation,
                    ..MeshDrawOptions::new(area)
                };
                draw_textured_mesh(paint, mt, &md.positions, &muv, &ms.triangles, &opts);
            }
            let w = width as usize;
            let (pd, md) = (paint.data(), mask.data_mut());
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    let at = y as usize * w + x as usize;
                    let a = pd[at * 4 + 3];
                    md[at] = if s.invert_mask { 255 - a } else { a };
                }
            }
            Some(&*mask)
        };
        let opts = CompositeOptions {
            blend: blend_mode(s.blend),
            opacity: 1.0,
            offset: (0, 0),
            region: Some(area),
            alpha_lock: false,
        };
        composite_pixmap(&mut out, layer, &opts, mask);
    }
    out
}
