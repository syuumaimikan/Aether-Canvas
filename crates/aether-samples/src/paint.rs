//! A small vector painter for sample artwork.
//!
//! Paths of lines and Bézier curves are filled with exact anti-aliasing
//! (signed-area accumulation, as font rasterisers do), stroked with widths
//! that taper along their length, or blurred into soft shapes. Paint is a
//! solid colour or a linear or radial gradient, and compositing can be
//! restricted to what is already painted — which is how cel shadows and
//! highlights stay inside the part they shade.
//!
//! Painting happens on a premultiplied floating-point [`Canvas`] that
//! converts to a layer [`Pixmap`] when done.

use aether_core::color::Rgba8;
use aether_core::math::Vec2;
use aether_raster::Pixmap;

/// A colour with straight alpha, components in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// From `0xRRGGBB`, opaque.
    pub const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// The same colour at opacity `a`.
    pub const fn alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Linear mix toward `other`.
    pub fn mix(self, other: Color, t: f32) -> Self {
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// What fills a shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    /// Colour stops `(t, colour)` from `from` (t = 0) to `to` (t = 1).
    Linear {
        from: Vec2,
        to: Vec2,
        stops: Vec<(f32, Color)>,
    },
    /// Colour stops from `center` (t = 0) out to the ellipse of radii
    /// `radius` (t = 1).
    Radial {
        center: Vec2,
        radius: Vec2,
        stops: Vec<(f32, Color)>,
    },
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
    }
}

fn sample_stops(stops: &[(f32, Color)], t: f32) -> Color {
    let Some(first) = stops.first() else {
        return Color::hex(0).alpha(0.0);
    };
    if t <= first.0 {
        return first.1;
    }
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let span = (w[1].0 - w[0].0).max(1e-6);
            return w[0].1.mix(w[1].1, (t - w[0].0) / span);
        }
    }
    stops[stops.len() - 1].1
}

impl Paint {
    /// Two-stop vertical gradient from `top` at `y0` to `bottom` at `y1`.
    pub fn vertical(y0: f32, top: Color, y1: f32, bottom: Color) -> Self {
        Paint::Linear {
            from: Vec2::new(0.0, y0),
            to: Vec2::new(0.0, y1),
            stops: vec![(0.0, top), (1.0, bottom)],
        }
    }

    fn at(&self, p: Vec2) -> Color {
        match self {
            Paint::Solid(c) => *c,
            Paint::Linear { from, to, stops } => {
                let d = *to - *from;
                let t = (p - *from).dot(d) / d.length_squared().max(1e-6);
                sample_stops(stops, t.clamp(0.0, 1.0))
            }
            Paint::Radial {
                center,
                radius,
                stops,
            } => {
                let q = Vec2::new(
                    (p.x - center.x) / radius.x.max(1e-6),
                    (p.y - center.y) / radius.y.max(1e-6),
                );
                sample_stops(stops, q.length().clamp(0.0, 1.0))
            }
        }
    }
}

/// How paint combines with what is already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On top.
    Over,
    /// On top, but only where something is already painted.
    Atop,
    /// Multiply, only where something is painted (cel shadows).
    Multiply,
    /// Screen, only where something is painted (highlights).
    Screen,
    /// Remove paint.
    Erase,
}

/// A shape made of closed or open polylines, built from lines and curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    subpaths: Vec<(Vec<Vec2>, bool)>,
}

/// Segments per Bézier curve.
const CURVE_STEPS: usize = 24;

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    fn current(&mut self) -> &mut Vec<Vec2> {
        if self.subpaths.is_empty() {
            self.subpaths.push((Vec::new(), false));
        }
        &mut self.subpaths.last_mut().expect("a subpath").0
    }

    fn last(&self) -> Vec2 {
        self.subpaths
            .last()
            .and_then(|(p, _)| p.last())
            .copied()
            .unwrap_or(Vec2::ZERO)
    }

    /// Start a new subpath at `(x, y)`.
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.subpaths.push((vec![Vec2::new(x, y)], false));
        self
    }

    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.current().push(Vec2::new(x, y));
        self
    }

    /// Quadratic Bézier through control point `(cx, cy)`.
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        let p0 = self.last();
        let (c, p1) = (Vec2::new(cx, cy), Vec2::new(x, y));
        let points = self.current();
        for i in 1..=CURVE_STEPS {
            let t = i as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            points.push(p0 * (u * u) + c * (2.0 * u * t) + p1 * (t * t));
        }
        self
    }

    /// Cubic Bézier through control points `c1` and `c2`.
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        let p0 = self.last();
        let (c1, c2, p1) = (Vec2::new(c1x, c1y), Vec2::new(c2x, c2y), Vec2::new(x, y));
        let points = self.current();
        for i in 1..=CURVE_STEPS {
            let t = i as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            points
                .push(p0 * (u * u * u) + c1 * (3.0 * u * u * t) + c2 * (3.0 * u * t * t) + p1 * (t * t * t));
        }
        self
    }

    /// A smooth curve through `points` (Catmull-Rom), continuing the
    /// current subpath.
    pub fn smooth_through(mut self, points: &[(f32, f32)]) -> Self {
        let mut all = vec![self.last()];
        all.extend(points.iter().map(|&(x, y)| Vec2::new(x, y)));
        let n = all.len();
        for i in 0..n - 1 {
            let p0 = all[i.saturating_sub(1)];
            let (p1, p2) = (all[i], all[i + 1]);
            let p3 = all[(i + 2).min(n - 1)];
            let c1 = p1 + (p2 - p0) * (1.0 / 6.0);
            let c2 = p2 - (p3 - p1) * (1.0 / 6.0);
            self = self.cubic_to(c1.x, c1.y, c2.x, c2.y, p2.x, p2.y);
        }
        self
    }

    /// Close the current subpath.
    pub fn close(mut self) -> Self {
        if let Some(last) = self.subpaths.last_mut() {
            last.1 = true;
        }
        self
    }

    /// A closed ellipse.
    pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Self {
        let n = 96;
        let points = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Vec2::new(cx + rx * a.cos(), cy + ry * a.sin())
            })
            .collect();
        Self {
            subpaths: vec![(points, true)],
        }
    }

    /// A closed polygon.
    pub fn polygon(points: &[(f32, f32)]) -> Self {
        Self {
            subpaths: vec![(points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(), true)],
        }
    }

    /// Every subpath of `other` added to this one.
    pub fn with(mut self, other: Path) -> Self {
        self.subpaths.extend(other.subpaths);
        self
    }

    /// Mirror about the vertical line `x = axis`.
    pub fn mirrored(&self, axis: f32) -> Self {
        Self {
            subpaths: self
                .subpaths
                .iter()
                .map(|(p, closed)| {
                    (
                        p.iter().rev().map(|v| Vec2::new(2.0 * axis - v.x, v.y)).collect(),
                        *closed,
                    )
                })
                .collect(),
        }
    }

    /// Move by `(dx, dy)`.
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            subpaths: self
                .subpaths
                .iter()
                .map(|(p, closed)| (p.iter().map(|v| Vec2::new(v.x + dx, v.y + dy)).collect(), *closed))
                .collect(),
        }
    }

    /// The polylines, each closed polygon repeating its first point.
    fn polylines(&self) -> impl Iterator<Item = (&[Vec2], bool)> {
        self.subpaths
            .iter()
            .filter(|(p, _)| p.len() >= 2)
            .map(|(p, c)| (p.as_slice(), *c))
    }

    fn bounds(&self) -> Option<(Vec2, Vec2)> {
        let mut it = self.subpaths.iter().flat_map(|(p, _)| p.iter());
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
    }
}

/// How a stroke's width varies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// Full width, pixels.
    pub width: f32,
    /// Fraction of the length over which the start swells from nothing.
    pub taper_in: f32,
    /// Fraction of the length over which the end thins to nothing.
    pub taper_out: f32,
}

impl Stroke {
    /// A stroke of constant width with round ends.
    pub fn even(width: f32) -> Self {
        Self {
            width,
            taper_in: 0.0,
            taper_out: 0.0,
        }
    }

    /// A pen line: thin at both ends.
    pub fn pen(width: f32) -> Self {
        Self {
            width,
            taper_in: 0.25,
            taper_out: 0.35,
        }
    }

    fn width_at(&self, t: f32) -> f32 {
        let ease = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        let mut w = self.width;
        if self.taper_in > 0.0 {
            w *= 0.15 + 0.85 * ease(t / self.taper_in);
        }
        if self.taper_out > 0.0 {
            w *= 0.15 + 0.85 * ease((1.0 - t) / self.taper_out);
        }
        w
    }
}

/// Premultiplied RGBA, `0..=1`.
type Px = [f32; 4];

/// Something to paint on.
pub struct Canvas {
    width: u32,
    height: u32,
    px: Vec<Px>,
}

/// Coverage of a region: `origin`, size and values in `0..=1`.
struct Coverage {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    values: Vec<f32>,
}

fn accumulate_line(acc: &mut [f32], stride: usize, h: usize, p0: Vec2, p1: Vec2) {
    if (p0.y - p1.y).abs() < 1e-9 {
        return;
    }
    let (dir, p0, p1) = if p0.y < p1.y {
        (1.0, p0, p1)
    } else {
        (-1.0, p1, p0)
    };
    let dxdy = (p1.x - p0.x) / (p1.y - p0.y);
    let mut x = p0.x;
    let y_start = p0.y.max(0.0);
    if p0.y < 0.0 {
        x -= p0.y * dxdy;
    }
    let y_end = (p1.y.ceil() as usize).min(h);
    let limit = (stride - 2) as f32;
    for y in (y_start as usize)..y_end {
        let row = y * stride;
        let dy = ((y + 1) as f32).min(p1.y) - (y as f32).max(p0.y);
        let x_next = x + dxdy * dy;
        let d = dy * dir;
        let (xa, xb) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let (xa, xb) = (xa.clamp(0.0, limit), xb.clamp(0.0, limit));
        let xa_floor = xa.floor();
        let xa_i = xa_floor as usize;
        let xb_ceil = xb.ceil();
        let xb_i = xb_ceil as usize;
        if xb_i <= xa_i + 1 {
            let xmf = 0.5 * (xa + xb) - xa_floor;
            acc[row + xa_i] += d - d * xmf;
            acc[row + xa_i + 1] += d * xmf;
        } else {
            let s = 1.0 / (xb - xa);
            let xa_f = xa - xa_floor;
            let a0 = 0.5 * s * (1.0 - xa_f) * (1.0 - xa_f);
            let xb_f = xb - xb_ceil + 1.0;
            let am = 0.5 * s * xb_f * xb_f;
            acc[row + xa_i] += d * a0;
            if xb_i == xa_i + 2 {
                acc[row + xa_i + 1] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - xa_f);
                acc[row + xa_i + 1] += d * (a1 - a0);
                for xi in xa_i + 2..xb_i - 1 {
                    acc[row + xi] += d * s;
                }
                let a2 = a1 + (xb_i - xa_i - 3) as f32 * s;
                acc[row + xb_i - 1] += d * (1.0 - a2 - am);
            }
            acc[row + xb_i] += d * am;
        }
        x = x_next;
    }
}

impl Coverage {
    /// Rasterise closed polygons (open ones are closed implicitly).
    fn of(polygons: &[Vec<Vec2>], margin: f32, width: u32, height: u32) -> Option<Self> {
        let mut lo = Vec2::new(f32::MAX, f32::MAX);
        let mut hi = Vec2::new(f32::MIN, f32::MIN);
        for p in polygons.iter().flatten() {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        if lo.x > hi.x {
            return None;
        }
        let x0 = ((lo.x - margin).floor() as i32 - 1).max(0);
        let y0 = ((lo.y - margin).floor() as i32 - 1).max(0);
        let x1 = ((hi.x + margin).ceil() as i32 + 1).min(width as i32);
        let y1 = ((hi.y + margin).ceil() as i32 + 1).min(height as i32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let stride = w + 2;
        let mut acc = vec![0.0f32; stride * h];
        let origin = Vec2::new(x0 as f32, y0 as f32);
        for poly in polygons {
            if poly.len() < 3 {
                continue;
            }
            for i in 0..poly.len() {
                let a = poly[i] - origin;
                let b = poly[(i + 1) % poly.len()] - origin;
                accumulate_line(&mut acc, stride, h, a, b);
            }
        }
        let mut values = vec![0.0; w * h];
        for y in 0..h {
            let mut sum = 0.0f32;
            for x in 0..w {
                sum += acc[y * stride + x];
                values[y * w + x] = sum.abs().min(1.0);
            }
        }
        Some(Self { x0, y0, w, h, values })
    }

    fn blur(&mut self, sigma: f32) {
        if sigma <= 0.0 {
            return;
        }
        let r = (sigma * 3.0).ceil() as i32;
        let kernel: Vec<f32> = (-r..=r)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum: f32 = kernel.iter().sum();
        let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
        let (w, h) = (self.w as i32, self.h as i32);
        let mut tmp = vec![0.0; self.values.len()];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let sx = x + k as i32 - r;
                    if (0..w).contains(&sx) {
                        acc += self.values[(y * w + sx) as usize] * weight;
                    }
                }
                tmp[(y * w + x) as usize] = acc;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let sy = y + k as i32 - r;
                    if (0..h).contains(&sy) {
                        acc += tmp[(sy * w + x) as usize] * weight;
                    }
                }
                self.values[(y * w + x) as usize] = acc;
            }
        }
    }
}

/// Offset outline of a polyline with tapering width, as one polygon.
fn stroke_polygon(points: &[Vec2], closed: bool, style: &Stroke) -> Vec<Vec<Vec2>> {
    let mut pts: Vec<Vec2> = Vec::with_capacity(points.len() + 1);
    for p in points {
        if pts.last().is_none_or(|q: &Vec2| q.distance(*p) > 1e-3) {
            pts.push(*p);
        }
    }
    if closed && pts.len() > 2 {
        pts.push(pts[0]);
    }
    if pts.len() < 2 {
        return Vec::new();
    }
    // Resample so the width profile has points to live on.
    let raw_length: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let step = (raw_length / 64.0).max(1.5);
    let mut fine = vec![pts[0]];
    for w in pts.windows(2) {
        let n = (w[0].distance(w[1]) / step).ceil().max(1.0) as usize;
        for k in 1..=n {
            fine.push(w[0] + (w[1] - w[0]) * (k as f32 / n as f32));
        }
    }
    let pts = fine;
    let mut lengths = vec![0.0f32];
    for w in pts.windows(2) {
        lengths.push(lengths.last().unwrap() + w[0].distance(w[1]));
    }
    let total = lengths.last().copied().unwrap_or(1.0).max(1e-3);
    let n = pts.len();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    let normal = |a: Vec2, b: Vec2| {
        let d = (b - a).normalized();
        Vec2::new(-d.y, d.x)
    };
    for i in 0..n {
        let before = if i > 0 {
            Some(normal(pts[i - 1], pts[i]))
        } else {
            None
        };
        let after = if i + 1 < n {
            Some(normal(pts[i], pts[i + 1]))
        } else {
            None
        };
        let nrm = match (before, after) {
            (Some(a), Some(b)) => {
                let m = (a + b).normalized();
                let cos = m.dot(a).max(0.5);
                m * (1.0 / cos)
            }
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => Vec2::new(0.0, 1.0),
        };
        let w = if closed {
            style.width
        } else {
            style.width_at(lengths[i] / total)
        } * 0.5;
        left.push(pts[i] + nrm * w);
        right.push(pts[i] - nrm * w);
    }
    let mut polys = Vec::new();
    let mut outline = left;
    outline.extend(right.into_iter().rev());
    polys.push(outline);
    if !closed {
        // Round caps.
        for (i, t) in [(0usize, 0.0f32), (n - 1, 1.0)] {
            let r = style.width_at(t) * 0.5;
            if r > 0.3 {
                polys.push(
                    (0..16)
                        .map(|k| {
                            let a = k as f32 / 16.0 * std::f32::consts::TAU;
                            pts[i] + Vec2::new(a.cos(), a.sin()) * r
                        })
                        .collect(),
                );
            }
        }
    }
    polys
}

impl Canvas {
    /// A transparent canvas.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            px: vec![[0.0; 4]; (width * height) as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn composite(&mut self, cov: &Coverage, paint: &Paint, mode: Mode) {
        for y in 0..cov.h {
            let py = cov.y0 + y as i32;
            for x in 0..cov.w {
                let c = cov.values[y * cov.w + x];
                if c <= 0.0 {
                    continue;
                }
                let px = cov.x0 + x as i32;
                let color = paint.at(Vec2::new(px as f32 + 0.5, py as f32 + 0.5));
                let sa = (color.a * c).clamp(0.0, 1.0);
                if sa <= 0.0 {
                    continue;
                }
                let d = &mut self.px[(py as u32 * self.width + px as u32) as usize];
                let src = [color.r * sa, color.g * sa, color.b * sa, sa];
                match mode {
                    Mode::Over => {
                        for k in 0..4 {
                            d[k] = src[k] + d[k] * (1.0 - sa);
                        }
                    }
                    Mode::Atop => {
                        let da = d[3];
                        for k in 0..3 {
                            d[k] = src[k] * da + d[k] * (1.0 - sa);
                        }
                    }
                    Mode::Multiply => {
                        let tint = [color.r, color.g, color.b];
                        for k in 0..3 {
                            d[k] = d[k] * (1.0 - sa) + d[k] * tint[k] * sa;
                        }
                    }
                    Mode::Screen => {
                        let da = d[3];
                        let tint = [color.r, color.g, color.b];
                        for k in 0..3 {
                            let screened = d[k] + tint[k] * da - d[k] * tint[k];
                            d[k] = d[k] * (1.0 - sa) + screened * sa;
                        }
                    }
                    Mode::Erase => {
                        for v in d.iter_mut() {
                            *v *= 1.0 - sa;
                        }
                    }
                }
            }
        }
    }

    /// Fill `path` (every subpath closed).
    pub fn fill(&mut self, path: &Path, paint: impl Into<Paint>, mode: Mode) {
        let polys: Vec<Vec<Vec2>> = path.polylines().map(|(p, _)| p.to_vec()).collect();
        if let Some(cov) = Coverage::of(&polys, 0.0, self.width, self.height) {
            self.composite(&cov, &paint.into(), mode);
        }
    }

    /// Fill `path` blurred by `sigma` pixels: soft blush, glow, shadow.
    pub fn soft(&mut self, path: &Path, sigma: f32, paint: impl Into<Paint>, mode: Mode) {
        let polys: Vec<Vec<Vec2>> = path.polylines().map(|(p, _)| p.to_vec()).collect();
        if let Some(mut cov) = Coverage::of(&polys, sigma * 3.0, self.width, self.height) {
            cov.blur(sigma);
            self.composite(&cov, &paint.into(), mode);
        }
    }

    /// Stroke every subpath of `path`.
    pub fn stroke(&mut self, path: &Path, style: Stroke, paint: impl Into<Paint>, mode: Mode) {
        let paint = paint.into();
        // One coverage for the whole path, so overlapping pieces do not
        // darken where they meet.
        let mut polys = Vec::new();
        for (points, closed) in path.polylines() {
            polys.extend(stroke_polygon(points, closed, &style));
        }
        // Pieces are rasterised separately and combined by maximum, since
        // stroke outlines may cross themselves.
        let mut merged: Option<Coverage> = None;
        let Some((lo, hi)) = path.bounds() else { return };
        let margin = style.width;
        let region = Path::polygon(&[
            (lo.x - margin, lo.y - margin),
            (hi.x + margin, lo.y - margin),
            (hi.x + margin, hi.y + margin),
            (lo.x - margin, hi.y + margin),
        ]);
        let region_polys: Vec<Vec<Vec2>> = region.polylines().map(|(p, _)| p.to_vec()).collect();
        if let Some(mut base) = Coverage::of(&region_polys, 0.0, self.width, self.height) {
            base.values.iter_mut().for_each(|v| *v = 0.0);
            merged = Some(base);
        }
        let Some(mut merged) = merged else { return };
        for poly in polys {
            if let Some(c) = Coverage::of(std::slice::from_ref(&poly), 0.0, self.width, self.height) {
                for y in 0..c.h {
                    for x in 0..c.w {
                        let (gx, gy) = (c.x0 + x as i32 - merged.x0, c.y0 + y as i32 - merged.y0);
                        if gx >= 0 && gy >= 0 && (gx as usize) < merged.w && (gy as usize) < merged.h {
                            let m = &mut merged.values[gy as usize * merged.w + gx as usize];
                            *m = m.max(c.values[y * c.w + x]);
                        }
                    }
                }
            }
        }
        self.composite(&merged, &paint, mode);
    }

    /// Opacity of every pixel, for checks.
    pub fn alpha_at(&self, x: u32, y: u32) -> f32 {
        self.px[(y * self.width + x) as usize][3]
    }

    /// The painting as a layer pixmap (straight alpha).
    pub fn to_pixmap(&self) -> Pixmap {
        let mut pm = Pixmap::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let p = self.px[(y * self.width + x) as usize];
                let a = p[3].clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let c = |v: f32| ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8;
                pm.set(
                    x as i32,
                    y as i32,
                    Rgba8::new(c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8),
                );
            }
        }
        pm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage_sum(c: &Canvas) -> f32 {
        c.px.iter().map(|p| p[3]).sum()
    }

    #[test]
    fn filled_shapes_cover_their_area() {
        let mut c = Canvas::new(100, 100);
        c.fill(
            &Path::polygon(&[(10.5, 10.25), (60.5, 10.25), (60.5, 40.25), (10.5, 40.25)]),
            Color::hex(0xff0000),
            Mode::Over,
        );
        assert!((coverage_sum(&c) - 1500.0).abs() < 0.5, "{}", coverage_sum(&c));
        let mut c = Canvas::new(100, 100);
        c.fill(
            &Path::ellipse(50.0, 50.0, 30.0, 20.0),
            Color::hex(0xff0000),
            Mode::Over,
        );
        let area = std::f32::consts::PI * 600.0;
        assert!(
            (coverage_sum(&c) - area).abs() < area * 0.01,
            "{}",
            coverage_sum(&c)
        );
        // Edges are anti-aliased.
        assert!(c.alpha_at(50, 30) > 0.0 && c.alpha_at(50, 30) < 1.0 || c.alpha_at(50, 29) > 0.0);
    }

    #[test]
    fn atop_and_multiply_stay_inside_the_paint() {
        let mut c = Canvas::new(40, 40);
        c.fill(
            &Path::ellipse(20.0, 20.0, 10.0, 10.0),
            Color::hex(0xffffff),
            Mode::Over,
        );
        let before = coverage_sum(&c);
        c.fill(
            &Path::polygon(&[(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)]),
            Color::hex(0x808080),
            Mode::Multiply,
        );
        c.fill(
            &Path::polygon(&[(0.0, 20.0), (40.0, 20.0), (40.0, 40.0), (0.0, 40.0)]),
            Color::hex(0xff0000),
            Mode::Atop,
        );
        assert!((coverage_sum(&c) - before).abs() < 1e-3, "alpha unchanged");
        let top = c.to_pixmap().get(20, 14);
        assert!(top.r < 140 && top.r > 110, "{top:?}");
        let bottom = c.to_pixmap().get(20, 26);
        assert_eq!((bottom.r, bottom.g), (255, 0));
    }

    #[test]
    fn tapered_strokes_thin_toward_their_ends() {
        let mut c = Canvas::new(120, 40);
        let line = Path::new().move_to(10.0, 20.0).line_to(110.0, 20.0);
        c.stroke(&line, Stroke::pen(8.0), Color::hex(0), Mode::Over);
        let thickness = |x: u32| (0..40).map(|y| c.alpha_at(x, y)).sum::<f32>();
        assert!(thickness(60) > 7.0, "{}", thickness(60));
        assert!(thickness(12) < thickness(60) * 0.5);
        assert!(thickness(108) < thickness(60) * 0.5);
    }

    #[test]
    fn soft_shapes_fade_out() {
        let mut c = Canvas::new(80, 80);
        c.soft(
            &Path::ellipse(40.0, 40.0, 15.0, 15.0),
            4.0,
            Color::hex(0xff0000),
            Mode::Over,
        );
        assert!(c.alpha_at(40, 40) > 0.95);
        let edge = c.alpha_at(55, 40);
        assert!(edge > 0.2 && edge < 0.8, "{edge}");
    }
}
