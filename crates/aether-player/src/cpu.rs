//! A software renderer for players.
//!
//! This is the reference every GPU renderer is checked against, and it is
//! handy on its own for thumbnails, servers and tests. It uses the same
//! triangle rasteriser and blend kernels as the editor's compositor.

use crate::model::BlendKind;
use crate::player::{DrawItem, Player};
use aether_core::blend::BlendMode;
use aether_core::math::{IRect, Rect, Vec2};
use aether_raster::composite::{composite_pixmap, CompositeOptions};
use aether_raster::mesh::{draw_textured_mesh, MeshDrawOptions};
use aether_raster::transform::Interpolation;
use aether_raster::{Mask, Pixmap};

impl From<BlendKind> for BlendMode {
    fn from(kind: BlendKind) -> Self {
        match kind {
            BlendKind::Normal => BlendMode::Normal,
            BlendKind::Multiply => BlendMode::Multiply,
            BlendKind::Screen => BlendMode::Screen,
            BlendKind::Add => BlendMode::Add,
        }
    }
}

/// Draw the player's current pose into a new canvas-sized buffer.
///
/// `textures` are the decoded texture pages, in model order.
pub fn render(player: &Player, textures: &[Pixmap]) -> Pixmap {
    let model = player.model();
    let mut target = Pixmap::new(model.width, model.height);
    render_into(player, textures, &mut target);
    target
}

/// Draw the player's current pose over `target` (canvas-sized).
pub fn render_into(player: &Player, textures: &[Pixmap], target: &mut Pixmap) {
    for item in player.draw_list() {
        if item.opacity <= 0.0 {
            continue;
        }
        let Some(layer) = draw_part(player, textures, item, item.part as usize, 1.0, target.bounds()) else {
            continue;
        };
        let mask = match item.mask_part() {
            Some(base) => {
                let mut mask = Mask::new(target.width(), target.height());
                if let Some(coverage) =
                    draw_part(player, textures, item, base, item.mask_opacity, target.bounds())
                {
                    stamp_alpha(&mut mask, &coverage);
                }
                Some(mask)
            }
            None => None,
        };
        let opts = CompositeOptions {
            blend: item.blend_kind().into(),
            opacity: item.opacity,
            offset: (layer.at.x, layer.at.y),
            region: None,
            alpha_lock: false,
        };
        composite_pixmap(target, &layer.pixels, &opts, mask.as_ref());
    }
}

/// A part drawn into a buffer covering just its bounds.
struct Drawn {
    pixels: Pixmap,
    at: IRect,
}

fn draw_part(
    player: &Player,
    textures: &[Pixmap],
    item: &DrawItem,
    part: usize,
    opacity: f32,
    canvas: IRect,
) -> Option<Drawn> {
    let info = player.model().parts.get(part)?;
    let texture = textures.get(info.texture as usize)?;
    let flat = player.positions(part);
    let positions: Vec<Vec2> = flat.chunks_exact(2).map(|c| Vec2::new(c[0], c[1])).collect();
    let bounds = positions
        .iter()
        .fold(None::<Rect>, |acc, &p| {
            let r = Rect::from_corners(p, p);
            Some(acc.map_or(r, |a| a.union(&r)))
        })?
        .to_irect_outer()
        .expanded(1)
        .intersect(&canvas);
    if bounds.is_empty() {
        return None;
    }
    let shift = Vec2::new(bounds.x as f32, bounds.y as f32);
    let local: Vec<Vec2> = positions.iter().map(|&p| p - shift).collect();
    let size = Vec2::new(texture.width() as f32, texture.height() as f32);
    let uvs: Vec<Vec2> = info
        .uvs
        .iter()
        .map(|uv| Vec2::new(uv.x * size.x, uv.y * size.y))
        .collect();
    let mut pixels = Pixmap::new(bounds.width as u32, bounds.height as u32);
    // The mask draw of a base part is untinted; the part itself is tinted.
    let tinted = item.part as usize == part;
    let opts = MeshDrawOptions {
        region: pixels.bounds(),
        opacity,
        multiply: if tinted { item.multiply } else { [1.0; 3] },
        screen: if tinted { item.screen } else { [0.0; 3] },
        interpolation: Interpolation::Bilinear,
    };
    draw_textured_mesh(&mut pixels, texture, &local, &uvs, &info.triangles, &opts);
    Some(Drawn { pixels, at: bounds })
}

fn stamp_alpha(mask: &mut Mask, drawn: &Drawn) {
    for y in 0..drawn.at.height {
        for x in 0..drawn.at.width {
            let a = drawn.pixels.get(x, y).a;
            if a > 0 {
                mask.set(drawn.at.x + x, drawn.at.y + y, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Model, Node, Part, Texture};
    use aether_core::color::Rgba8;
    use aether_core::LayerId;

    fn quad(layer: u64, rect: [f32; 4], blend: BlendKind) -> Part {
        let [x, y, w, h] = rect;
        Part {
            layer: LayerId(layer),
            name: String::new(),
            texture: 0,
            vertices: vec![
                Vec2::new(x, y),
                Vec2::new(x + w, y),
                Vec2::new(x + w, y + h),
                Vec2::new(x, y + h),
            ],
            // Sample the middle of a 4×4 block of one colour.
            uvs: vec![Vec2::new(0.5 * (layer - 1) as f32 + 0.25, 0.5); 4],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            opacity: 1.0,
            blend,
            transform: None,
        }
    }

    #[test]
    fn clipped_parts_only_show_inside_their_base() {
        // Texture: a red block then a blue block.
        let mut texture = Pixmap::new(8, 1);
        for x in 0..8 {
            let c = if x < 4 {
                Rgba8::new(255, 0, 0, 255)
            } else {
                Rgba8::new(0, 0, 255, 255)
            };
            texture.set(x, 0, c);
        }
        let mut model = Model::new("clip", 20, 10);
        model.textures.push(Texture {
            file: "t.png".into(),
            width: 8,
            height: 1,
        });
        model
            .parts
            .push(quad(1, [0.0, 0.0, 10.0, 10.0], BlendKind::Normal));
        model
            .parts
            .push(quad(2, [5.0, 0.0, 15.0, 10.0], BlendKind::Normal));
        model.tree.push(Node::Part {
            index: 0,
            part: 0,
            clipped: vec![1],
        });
        let player = Player::new(model).unwrap();
        let out = render(&player, &[texture]);
        assert_eq!(out.get(2, 5), Rgba8::new(255, 0, 0, 255), "base only");
        assert_eq!(out.get(7, 5), Rgba8::new(0, 0, 255, 255), "clipped over base");
        assert_eq!(out.get(15, 5).a, 0, "clipped part outside the base");
    }

    #[test]
    fn multiply_parts_darken_what_is_below() {
        let mut texture = Pixmap::new(8, 1);
        for x in 0..8 {
            let c = if x < 4 {
                Rgba8::new(200, 200, 200, 255)
            } else {
                Rgba8::new(128, 255, 255, 255)
            };
            texture.set(x, 0, c);
        }
        let mut model = Model::new("multiply", 10, 10);
        model.textures.push(Texture {
            file: "t.png".into(),
            width: 8,
            height: 1,
        });
        model
            .parts
            .push(quad(1, [0.0, 0.0, 10.0, 10.0], BlendKind::Normal));
        model
            .parts
            .push(quad(2, [0.0, 0.0, 10.0, 10.0], BlendKind::Multiply));
        model.tree.push(Node::Part {
            index: 0,
            part: 0,
            clipped: vec![],
        });
        model.tree.push(Node::Part {
            index: 1,
            part: 1,
            clipped: vec![],
        });
        let out = render(&Player::new(model).unwrap(), &[texture]);
        let px = out.get(5, 5);
        assert!((px.r as i32 - 100).abs() <= 1, "{px:?}");
        assert_eq!(px.g, 200);
    }
}
