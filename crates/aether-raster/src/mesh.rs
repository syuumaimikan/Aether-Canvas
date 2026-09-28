//! Textured triangle meshes.
//!
//! Rigging deforms artwork by moving the vertices of a triangle mesh laid over
//! a layer, then redrawing the layer's pixels through the moved triangles.
//! This module is that redraw: every triangle carries an affine map from its
//! deformed position back to where its pixels came from, and each destination
//! pixel centre inside the triangle samples the texture through that map.
//!
//! Two properties matter more than raw speed:
//!
//! * **Watertight coverage.** Adjacent triangles share an edge, and a pixel
//!   centre lying exactly on that edge must be drawn by exactly one of them —
//!   otherwise a deformed layer shows hairline cracks or double-dark seams.
//!   Edge functions are evaluated in a canonical vertex order so the two
//!   triangles see bit-identical (negated) values, and ties are broken by a
//!   consistent top-left rule.
//! * **Silhouettes come from the texture, not the mesh.** Triangles are
//!   sampled at pixel centres with no geometric anti-aliasing; the soft edge
//!   of the artwork is the texture's own alpha, sampled bilinearly. Meshes are
//!   generated with a margin around the art, so the mesh outline never cuts
//!   through painted pixels.
//!
//! Work is split into horizontal bands processed in parallel; within a band
//! triangles are drawn in order, so folded meshes composite deterministically.

use crate::pixmap::{Pixmap, BYTES_PER_PIXEL};
use crate::transform::Interpolation;
use crate::Mask;
use aether_core::color::Rgba8;
use aether_core::math::{IRect, Vec2};
use rayon::prelude::*;

/// Rows per parallel work item.
const BAND_ROWS: i32 = 16;

/// How a mesh's texels are tinted and blended while drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshDrawOptions {
    /// Destination rectangle to limit drawing to.
    pub region: IRect,
    /// Opacity multiplier in `0..=1`.
    pub opacity: f32,
    /// Per-channel multiply colour (`[1, 1, 1]` leaves the texture alone).
    pub multiply: [f32; 3],
    /// Per-channel screen colour (`[0, 0, 0]` leaves the texture alone).
    pub screen: [f32; 3],
    /// Texture filtering.
    pub interpolation: Interpolation,
}

impl MeshDrawOptions {
    /// Untinted, fully opaque drawing limited to `region`.
    pub fn new(region: IRect) -> Self {
        Self {
            region,
            opacity: 1.0,
            multiply: [1.0; 3],
            screen: [0.0; 3],
            interpolation: Interpolation::Bilinear,
        }
    }

    fn is_untinted(&self) -> bool {
        self.multiply == [1.0; 3] && self.screen == [0.0; 3]
    }
}

/// A triangle prepared for scan conversion.
#[derive(Clone, Copy, Debug)]
struct PreparedTriangle {
    /// Destination bounds, already clipped to the draw region.
    bounds: IRect,
    /// The three directed edges as `(a, b, flip)` in canonical order.
    edges: [Edge; 3],
    /// Affine map from destination position to texture position.
    map: [f64; 6],
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    /// Canonical start point (the vertex with the lower index).
    a: (f64, f64),
    /// Canonical end point.
    b: (f64, f64),
    /// `1.0` when the triangle traverses the edge in canonical direction,
    /// `-1.0` when it traverses it backwards.
    sign: f64,
    /// Whether a pixel centre exactly on this edge belongs to the triangle.
    owns_ties: bool,
}

impl Edge {
    fn new(positions: &[Vec2], from: u32, to: u32) -> Self {
        let (lo, hi, sign) = if from < to {
            (from, to, 1.0)
        } else {
            (to, from, -1.0)
        };
        let a = positions[lo as usize];
        let b = positions[hi as usize];
        let a = (a.x as f64, a.y as f64);
        let b = (b.x as f64, b.y as f64);
        // The directed edge as the triangle walks it. Any rule that is
        // antisymmetric in direction works; this is the usual "top-left".
        let dx = (b.0 - a.0) * sign;
        let dy = (b.1 - a.1) * sign;
        let owns_ties = dy > 0.0 || (dy == 0.0 && dx < 0.0);
        Self {
            a,
            b,
            sign,
            owns_ties,
        }
    }

    /// Signed distance-like value; positive on the triangle's inside.
    #[inline]
    fn eval(&self, px: f64, py: f64) -> f64 {
        let canonical = (self.b.0 - self.a.0) * (py - self.a.1) - (self.b.1 - self.a.1) * (px - self.a.0);
        canonical * self.sign
    }

    #[inline]
    fn accepts(&self, px: f64, py: f64) -> bool {
        let e = self.eval(px, py);
        e > 0.0 || (e == 0.0 && self.owns_ties)
    }
}

/// Prepare every usable triangle: degenerate ones and ones with bad indices are
/// skipped rather than failing the whole draw.
fn prepare(positions: &[Vec2], uvs: &[Vec2], triangles: &[[u32; 3]], clip: IRect) -> Vec<PreparedTriangle> {
    let count = positions.len().min(uvs.len());
    let mut out = Vec::with_capacity(triangles.len());
    for tri in triangles {
        if tri.iter().any(|&i| i as usize >= count) {
            continue;
        }
        let [i0, i1, i2] = *tri;
        let p0 = positions[i0 as usize];
        let p1 = positions[i1 as usize];
        let p2 = positions[i2 as usize];
        if !(p0.is_finite() && p1.is_finite() && p2.is_finite()) {
            continue;
        }
        let area = (p1.x as f64 - p0.x as f64) * (p2.y as f64 - p0.y as f64)
            - (p1.y as f64 - p0.y as f64) * (p2.x as f64 - p0.x as f64);
        if area.abs() < 1e-9 {
            continue;
        }
        // Walk the triangle so its inside is where every edge is positive.
        let (j1, j2) = if area > 0.0 { (i1, i2) } else { (i2, i1) };
        let edges = [
            Edge::new(positions, i0, j1),
            Edge::new(positions, j1, j2),
            Edge::new(positions, j2, i0),
        ];
        // With y pointing down, "positive on the inside" for the edge function
        // above means the walk must be counter-clockwise in maths orientation;
        // flip every edge if we got the opposite convention.
        let probe = (
            (p0.x as f64 + p1.x as f64 + p2.x as f64) / 3.0,
            (p0.y as f64 + p1.y as f64 + p2.y as f64) / 3.0,
        );
        let edges = if edges[0].eval(probe.0, probe.1) < 0.0 {
            edges.map(|mut e| {
                e.sign = -e.sign;
                let dx = (e.b.0 - e.a.0) * e.sign;
                let dy = (e.b.1 - e.a.1) * e.sign;
                e.owns_ties = dy > 0.0 || (dy == 0.0 && dx < 0.0);
                e
            })
        } else {
            edges
        };

        let Some(map) = affine_between(
            [p0, p1, p2],
            [uvs[i0 as usize], uvs[i1 as usize], uvs[i2 as usize]],
        ) else {
            continue;
        };

        let min_x = p0.x.min(p1.x).min(p2.x).floor() as i32;
        let min_y = p0.y.min(p1.y).min(p2.y).floor() as i32;
        let max_x = p0.x.max(p1.x).max(p2.x).ceil() as i32;
        let max_y = p0.y.max(p1.y).max(p2.y).ceil() as i32;
        let bounds = IRect::from_bounds(min_x, min_y, max_x + 1, max_y + 1).intersect(&clip);
        if bounds.is_empty() {
            continue;
        }
        out.push(PreparedTriangle { bounds, edges, map });
    }
    out
}

/// The affine map taking triangle `from` onto triangle `to`, as
/// `[a, b, c, d, tx, ty]` with `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`.
fn affine_between(from: [Vec2; 3], to: [Vec2; 3]) -> Option<[f64; 6]> {
    let (x0, y0) = (from[0].x as f64, from[0].y as f64);
    let (e1x, e1y) = (from[1].x as f64 - x0, from[1].y as f64 - y0);
    let (e2x, e2y) = (from[2].x as f64 - x0, from[2].y as f64 - y0);
    let det = e1x * e2y - e2x * e1y;
    if det.abs() < 1e-12 {
        return None;
    }
    // Inverse of [e1 e2] (columns).
    let inv = [e2y / det, -e1y / det, -e2x / det, e1x / det];
    let (u0, v0) = (to[0].x as f64, to[0].y as f64);
    let (f1x, f1y) = (to[1].x as f64 - u0, to[1].y as f64 - v0);
    let (f2x, f2y) = (to[2].x as f64 - u0, to[2].y as f64 - v0);
    // M = [f1 f2] · inv
    let a = f1x * inv[0] + f2x * inv[1];
    let b = f1y * inv[0] + f2y * inv[1];
    let c = f1x * inv[2] + f2x * inv[3];
    let d = f1y * inv[2] + f2y * inv[3];
    let tx = u0 - (a * x0 + c * y0);
    let ty = v0 - (b * x0 + d * y0);
    Some([a, b, c, d, tx, ty])
}

impl PreparedTriangle {
    #[inline]
    fn contains(&self, px: f64, py: f64) -> bool {
        self.edges.iter().all(|e| e.accepts(px, py))
    }

    /// The columns of row `y` that can hold covered pixel centres, as
    /// `(x0, x1, inside0, inside1)`: pixels outside `x0..x1` are never
    /// covered, and pixels inside `inside0..inside1` are certainly covered.
    ///
    /// Each edge function is linear along the row. Its coefficients are
    /// differences of f32 inputs, exact in f64, so where it crosses zero is
    /// known to far better than a pixel: a pixel two columns clear of every
    /// crossing, on the inside, evaluates strictly positive, and one more
    /// than a column outside evaluates negative. Only the pixels in between
    /// need [`PreparedTriangle::contains`], which keeps the result exactly
    /// the bounding-box scan's, tie rule included.
    #[inline]
    fn row_span(&self, y: i32) -> (i32, i32, i32, i32) {
        let py = y as f64 + 0.5;
        let (mut lo, mut hi) = (self.bounds.x as f64, self.bounds.right() as f64);
        let (mut safe_lo, mut safe_hi) = (lo, hi);
        for e in &self.edges {
            // eval(px) = slope·px + offset.
            let slope = -(e.b.1 - e.a.1) * e.sign;
            if slope == 0.0 {
                // Constant along the row: one exact evaluation settles it.
                if !e.accepts(self.bounds.x as f64 + 0.5, py) {
                    return (0, 0, 0, 0);
                }
                continue;
            }
            let offset = ((e.b.0 - e.a.0) * (py - e.a.1) + (e.b.1 - e.a.1) * e.a.0) * e.sign;
            // Pixel x has its centre at x + 0.5.
            let cross = -offset / slope - 0.5;
            if slope > 0.0 {
                lo = lo.max(cross - 1.0);
                safe_lo = safe_lo.max(cross + 2.0);
            } else {
                hi = hi.min(cross + 2.0);
                safe_hi = safe_hi.min(cross - 1.0);
            }
        }
        let x0 = (lo.floor() as i32).max(self.bounds.x);
        let x1 = (hi.ceil() as i32).min(self.bounds.right()).max(x0);
        let inside0 = (safe_lo.ceil() as i32).clamp(x0, x1);
        let inside1 = (safe_hi.floor() as i32).clamp(inside0, x1);
        (x0, x1, inside0, inside1)
    }

    #[inline]
    fn texture_point(&self, px: f64, py: f64) -> (f32, f32) {
        let m = &self.map;
        (
            (m[0] * px + m[2] * py + m[4]) as f32,
            (m[1] * px + m[3] * py + m[5]) as f32,
        )
    }
}

/// Draw `texture` through a triangle mesh onto `dst`.
///
/// `positions` are where the vertices are now (destination pixels); `uvs` are
/// where the same vertices sit in `texture`, in texture pixels. Texels are
/// composited source-over, so a mesh that folds over itself stacks in
/// triangle order. Returns the destination rectangle that may have changed.
pub fn draw_textured_mesh(
    dst: &mut Pixmap,
    texture: &Pixmap,
    positions: &[Vec2],
    uvs: &[Vec2],
    triangles: &[[u32; 3]],
    opts: &MeshDrawOptions,
) -> IRect {
    let clip = opts.region.intersect(&dst.bounds());
    if clip.is_empty() || opts.opacity <= 0.0 {
        return IRect::EMPTY;
    }
    let prepared = prepare(positions, uvs, triangles, clip);
    if prepared.is_empty() {
        return IRect::EMPTY;
    }
    let touched = prepared.iter().fold(IRect::EMPTY, |acc, t| acc.union(&t.bounds));

    let width = dst.width() as usize;
    let stride = width * BYTES_PER_PIXEL;
    let first_row = touched.y;
    let last_row = touched.bottom();
    let opacity = opts.opacity.clamp(0.0, 1.0);
    let untinted = opts.is_untinted();
    let texels = Texels::new(texture);
    let data = dst.data_mut();
    let rows = &mut data[first_row as usize * stride..last_row as usize * stride];

    rows.par_chunks_mut(stride * BAND_ROWS as usize)
        .enumerate()
        .for_each(|(band, chunk)| {
            let band_top = first_row + band as i32 * BAND_ROWS;
            let band_bottom = band_top + (chunk.len() / stride) as i32;
            for tri in &prepared {
                let y0 = tri.bounds.y.max(band_top);
                let y1 = tri.bounds.bottom().min(band_bottom);
                if y0 >= y1 {
                    continue;
                }
                for y in y0..y1 {
                    let py = y as f64 + 0.5;
                    let row = &mut chunk[(y - band_top) as usize * stride..][..stride];
                    let (x0, x1, inside0, inside1) = tri.row_span(y);
                    for x in x0..x1 {
                        let px = x as f64 + 0.5;
                        if (x < inside0 || x >= inside1) && !tri.contains(px, py) {
                            continue;
                        }
                        let (u, v) = tri.texture_point(px, py);
                        let offset = x as usize * BYTES_PER_PIXEL;
                        let pixel = &mut row[offset..offset + BYTES_PER_PIXEL];
                        match opts.interpolation {
                            Interpolation::Bilinear => {
                                let texel = texels.bilinear(u, v);
                                blend_premultiplied(pixel, texel, opts, opacity, untinted);
                            }
                            Interpolation::Nearest => {
                                let texel = texture.sample_nearest(u, v);
                                if texel.a != 0 {
                                    blend_texel(pixel, texel, opts, opacity, untinted);
                                }
                            }
                        }
                    }
                }
            }
        });
    touched
}

/// Channel values `0..=255` as `0.0..=1.0`, so texels convert without a
/// division.
static UNIT: [f32; 256] = {
    let mut table = [0.0f32; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = i as f32 / 255.0;
        i += 1;
    }
    table
};

/// `floor` for the sampler's coordinates, without a libm call.
#[inline]
fn floor_i32(v: f32) -> i32 {
    let i = v as i32;
    if (i as f32) > v {
        i - 1
    } else {
        i
    }
}

/// Round a non-negative channel value to a byte.
#[inline]
fn to_byte(v: f32) -> u8 {
    (v + 0.5).clamp(0.0, 255.0) as u8
}

/// A texture read for the hot loop: bilinear sampling with the same texel
/// centres as [`Pixmap::sample_bilinear`], but returning premultiplied colour
/// in `0..=1` straight to the blender instead of rounding to a byte first.
struct Texels<'a> {
    data: &'a [u8],
    width: i32,
    height: i32,
    stride: usize,
}

impl<'a> Texels<'a> {
    fn new(texture: &'a Pixmap) -> Self {
        Self {
            data: texture.data(),
            width: texture.width() as i32,
            height: texture.height() as i32,
            stride: texture.width() as usize * BYTES_PER_PIXEL,
        }
    }

    #[inline]
    fn bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = floor_i32(fx);
        let y0 = floor_i32(fy);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let mut acc = [0.0f32; 4];
        let mut add = |px: i32, py: i32, w: f32| {
            if w <= 0.0 || px < 0 || py < 0 || px >= self.width || py >= self.height {
                return;
            }
            let o = py as usize * self.stride + px as usize * BYTES_PER_PIXEL;
            let texel = &self.data[o..o + BYTES_PER_PIXEL];
            let aw = UNIT[texel[3] as usize] * w;
            acc[0] += UNIT[texel[0] as usize] * aw;
            acc[1] += UNIT[texel[1] as usize] * aw;
            acc[2] += UNIT[texel[2] as usize] * aw;
            acc[3] += aw;
        };
        add(x0, y0, (1.0 - tx) * (1.0 - ty));
        add(x0 + 1, y0, tx * (1.0 - ty));
        add(x0, y0 + 1, (1.0 - tx) * ty);
        add(x0 + 1, y0 + 1, tx * ty);
        acc
    }
}

/// Tint a premultiplied sample and composite it source-over onto one
/// straight-alpha pixel. The same model as [`blend_texel`].
#[inline]
fn blend_premultiplied(
    pixel: &mut [u8],
    texel: [f32; 4],
    opts: &MeshDrawOptions,
    opacity: f32,
    untinted: bool,
) {
    let alpha = texel[3];
    // Below half a level the sample would have rounded to transparent.
    if alpha < 0.5 / 255.0 {
        return;
    }
    let straight = 1.0 / alpha;
    let mut r = texel[0] * straight;
    let mut g = texel[1] * straight;
    let mut b = texel[2] * straight;
    if !untinted {
        r *= opts.multiply[0];
        g *= opts.multiply[1];
        b *= opts.multiply[2];
        r = r + opts.screen[0] - r * opts.screen[0];
        g = g + opts.screen[1] - g * opts.screen[1];
        b = b + opts.screen[2] - b * opts.screen[2];
    }
    let sa = alpha * opacity;
    if sa <= 0.0 {
        return;
    }
    let da = UNIT[pixel[3] as usize];
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 1e-6 {
        return;
    }
    let dst_weight = da * (1.0 - sa);
    let scale = 255.0 / out_a;
    let mix = |s: f32, d: u8| to_byte((s * sa + UNIT[d as usize] * dst_weight) * scale);
    pixel[0] = mix(r, pixel[0]);
    pixel[1] = mix(g, pixel[1]);
    pixel[2] = mix(b, pixel[2]);
    pixel[3] = to_byte(out_a * 255.0);
}

/// Tint a texel and composite it source-over onto one straight-alpha pixel.
#[inline]
fn blend_texel(pixel: &mut [u8], texel: Rgba8, opts: &MeshDrawOptions, opacity: f32, untinted: bool) {
    let mut r = texel.r as f32 / 255.0;
    let mut g = texel.g as f32 / 255.0;
    let mut b = texel.b as f32 / 255.0;
    if !untinted {
        r *= opts.multiply[0];
        g *= opts.multiply[1];
        b *= opts.multiply[2];
        r = r + opts.screen[0] - r * opts.screen[0];
        g = g + opts.screen[1] - g * opts.screen[1];
        b = b + opts.screen[2] - b * opts.screen[2];
    }
    let sa = texel.a as f32 / 255.0 * opacity;
    if sa <= 0.0 {
        return;
    }
    let da = pixel[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 1e-6 {
        return;
    }
    let dst_weight = da * (1.0 - sa);
    let mix = |s: f32, d: u8| -> u8 {
        let d = d as f32 / 255.0;
        (((s * sa + d * dst_weight) / out_a) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    pixel[0] = mix(r, pixel[0]);
    pixel[1] = mix(g, pixel[1]);
    pixel[2] = mix(b, pixel[2]);
    pixel[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Which pixels of a `width × height` canvas a mesh covers, as a mask.
///
/// Used to check that a mesh really encloses the artwork it is meant to
/// carry, and to show mesh coverage in the editor.
pub fn mesh_coverage(width: u32, height: u32, positions: &[Vec2], triangles: &[[u32; 3]]) -> Mask {
    let mut mask = Mask::new(width, height);
    let clip = IRect::from_size(width, height);
    // Coverage does not depend on texture coordinates; reuse positions.
    for tri in prepare(positions, positions, triangles, clip) {
        for y in tri.bounds.y..tri.bounds.bottom() {
            let (x0, x1, inside0, inside1) = tri.row_span(y);
            for x in x0..x1 {
                if (x >= inside0 && x < inside1) || tri.contains(x as f64 + 0.5, y as f64 + 0.5) {
                    mask.set(x, y, 255);
                }
            }
        }
    }
    mask
}

/// How many times each pixel is covered, for testing watertightness.
#[cfg(test)]
fn coverage_counts(width: u32, height: u32, positions: &[Vec2], triangles: &[[u32; 3]]) -> Vec<u8> {
    let mut counts = vec![0u8; (width * height) as usize];
    let clip = IRect::from_size(width, height);
    for tri in prepare(positions, positions, triangles, clip) {
        for y in tri.bounds.y..tri.bounds.bottom() {
            for x in tri.bounds.x..tri.bounds.right() {
                if tri.contains(x as f64 + 0.5, y as f64 + 0.5) {
                    counts[(y as u32 * width + x as u32) as usize] += 1;
                }
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::vec2;

    fn grid_mesh(x0: f32, y0: f32, w: f32, h: f32, cols: usize, rows: usize) -> (Vec<Vec2>, Vec<[u32; 3]>) {
        let mut points = Vec::new();
        for r in 0..=rows {
            for c in 0..=cols {
                points.push(vec2(
                    x0 + w * c as f32 / cols as f32,
                    y0 + h * r as f32 / rows as f32,
                ));
            }
        }
        let mut tris = Vec::new();
        let stride = cols as u32 + 1;
        for r in 0..rows as u32 {
            for c in 0..cols as u32 {
                let i = r * stride + c;
                tris.push([i, i + 1, i + stride]);
                tris.push([i + 1, i + stride + 1, i + stride]);
            }
        }
        (points, tris)
    }

    fn gradient_texture(w: u32, h: u32) -> Pixmap {
        let mut pm = Pixmap::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                pm.set(
                    x,
                    y,
                    Rgba8::new((x * 7 % 256) as u8, (y * 5 % 256) as u8, 128, 255),
                );
            }
        }
        pm
    }

    #[test]
    fn an_undeformed_mesh_reproduces_its_texture_exactly() {
        let texture = gradient_texture(40, 30);
        let (points, tris) = grid_mesh(0.0, 0.0, 40.0, 30.0, 5, 4);
        let mut out = Pixmap::new(40, 30);
        let opts = MeshDrawOptions::new(out.bounds());
        draw_textured_mesh(&mut out, &texture, &points, &points, &tris, &opts);
        for y in 0..30 {
            for x in 0..40 {
                assert_eq!(out.get(x, y), texture.get(x, y), "pixel {x},{y} changed");
            }
        }
    }

    #[test]
    fn shared_edges_are_covered_exactly_once() {
        // Irregular vertex positions put pixel centres on and near edges.
        let (mut points, tris) = grid_mesh(0.5, 0.5, 31.0, 23.0, 7, 5);
        for (i, p) in points.iter_mut().enumerate() {
            let jitter = ((i * 37) % 11) as f32 * 0.13;
            p.x += jitter;
            p.y -= jitter * 0.5;
        }
        let counts = coverage_counts(40, 30, &points, &tris);
        let doubled = counts.iter().filter(|&&c| c > 1).count();
        assert_eq!(doubled, 0, "{doubled} pixels were covered twice");
        // An axis-aligned integer grid has pixel centres exactly on edges.
        let (points, tris) = grid_mesh(0.0, 0.0, 16.0, 16.0, 4, 4);
        let counts = coverage_counts(16, 16, &points, &tris);
        assert!(
            counts.iter().all(|&c| c == 1),
            "every pixel of an exact grid must be drawn once"
        );
    }

    #[test]
    fn moving_vertices_moves_the_pixels() {
        let mut texture = Pixmap::new(20, 20);
        texture.set(5, 5, Rgba8::new(255, 0, 0, 255));
        let (points, tris) = grid_mesh(0.0, 0.0, 20.0, 20.0, 2, 2);
        let moved: Vec<Vec2> = points.iter().map(|p| *p + vec2(4.0, 3.0)).collect();
        let mut out = Pixmap::new(30, 30);
        let opts = MeshDrawOptions::new(out.bounds());
        draw_textured_mesh(&mut out, &texture, &moved, &points, &tris, &opts);
        assert_eq!(out.get(9, 8), Rgba8::new(255, 0, 0, 255));
        assert_eq!(out.get(5, 5).a, 0);
    }

    #[test]
    fn tint_and_opacity_follow_the_multiply_screen_model() {
        let texture = Pixmap::filled(8, 8, Rgba8::new(200, 100, 50, 255));
        let (points, tris) = grid_mesh(0.0, 0.0, 8.0, 8.0, 1, 1);
        let mut out = Pixmap::new(8, 8);
        let opts = MeshDrawOptions {
            multiply: [0.5, 1.0, 1.0],
            screen: [0.0, 0.0, 1.0],
            opacity: 0.5,
            ..MeshDrawOptions::new(out.bounds())
        };
        draw_textured_mesh(&mut out, &texture, &points, &points, &tris, &opts);
        let px = out.get(3, 3);
        assert_eq!(px.r, 100, "red is multiplied by 0.5");
        assert_eq!(px.g, 100, "green is untouched");
        assert_eq!(px.b, 255, "screen with white saturates blue");
        assert!(
            (px.a as i32 - 128).abs() <= 1,
            "opacity halves alpha, got {}",
            px.a
        );
    }

    #[test]
    fn drawing_is_limited_to_the_region() {
        let texture = Pixmap::filled(10, 10, Rgba8::WHITE);
        let (points, tris) = grid_mesh(0.0, 0.0, 10.0, 10.0, 2, 2);
        let mut out = Pixmap::new(10, 10);
        let region = IRect::new(2, 2, 3, 3);
        let written = draw_textured_mesh(
            &mut out,
            &texture,
            &points,
            &points,
            &tris,
            &MeshDrawOptions::new(region),
        );
        assert!(region.contains(written.x, written.y));
        assert_eq!(out.get(0, 0).a, 0);
        assert_eq!(out.get(3, 3).a, 255);
        assert_eq!(out.get(6, 6).a, 0);
    }

    #[test]
    fn degenerate_and_invalid_triangles_are_skipped() {
        let texture = Pixmap::filled(4, 4, Rgba8::WHITE);
        let points = vec![vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(2.0, 2.0)];
        let mut out = Pixmap::new(4, 4);
        let opts = MeshDrawOptions::new(out.bounds());
        let written = draw_textured_mesh(
            &mut out,
            &texture,
            &points,
            &points,
            &[[0, 1, 2], [0, 1, 9]],
            &opts,
        );
        assert!(written.is_empty());
    }

    #[test]
    fn coverage_mask_matches_the_triangles() {
        let (points, tris) = grid_mesh(2.0, 2.0, 6.0, 6.0, 1, 1);
        let mask = mesh_coverage(12, 12, &points, &tris);
        assert_eq!(mask.get(4, 4), 255);
        assert_eq!(mask.get(0, 0), 0);
        assert_eq!(mask.get(9, 9), 0);
    }

    #[test]
    fn row_spans_agree_with_the_exact_test() {
        // Deterministic pseudo-random triangles: slivers, near-horizontal and
        // near-vertical edges, sub-pixel and large coordinates.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let clip = IRect::new(-50, -50, 400, 400);
        for case in 0..4000 {
            let scale = [3.0, 40.0, 300.0][case % 3];
            let mut p = [0.0f32; 6];
            for v in &mut p {
                *v = (next() * scale) as f32 + 0.25 * (case % 4) as f32;
            }
            if case % 5 == 0 {
                p[3] = p[1] + (next() * 1e-3) as f32; // nearly horizontal edge
            }
            if case % 7 == 0 {
                p[2] = p[0] + (next() * 1e-3) as f32; // nearly vertical edge
            }
            let positions = [vec2(p[0], p[1]), vec2(p[2], p[3]), vec2(p[4], p[5])];
            for tri in prepare(&positions, &positions, &[[0, 1, 2]], clip) {
                for y in tri.bounds.y..tri.bounds.bottom() {
                    let (x0, x1, inside0, inside1) = tri.row_span(y);
                    assert!(x0 <= inside0 && inside0 <= inside1 && inside1 <= x1);
                    for x in tri.bounds.x..tri.bounds.right() {
                        let covered = tri.contains(x as f64 + 0.5, y as f64 + 0.5);
                        if covered {
                            assert!(
                                x >= x0 && x < x1,
                                "case {case}: pixel ({x}, {y}) outside span {x0}..{x1}"
                            );
                        }
                        if x >= inside0 && x < inside1 {
                            assert!(covered, "case {case}: pixel ({x}, {y}) in the sure part {inside0}..{inside1} is not covered");
                        }
                    }
                }
            }
        }
    }
}
