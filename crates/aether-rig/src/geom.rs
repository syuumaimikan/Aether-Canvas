//! Geometry helpers for meshes: barycentric coordinates, point location and
//! Delaunay triangulation.
//!
//! Everything here works in `f64` internally. Triangulation predicates are
//! where floating-point error turns into visibly broken meshes (slivers,
//! overlapping triangles), and the cost of doubles is irrelevant at the sizes
//! a 2D rig deals with — a few thousand points at most.

use aether_core::math::Vec2;
use std::collections::{BTreeMap, HashSet};

/// Barycentric coordinates of `p` in triangle `(a, b, c)`, or `None` for a
/// degenerate triangle.
pub fn barycentric(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<[f32; 3]> {
    let (px, py) = (p.x as f64, p.y as f64);
    let (ax, ay) = (a.x as f64, a.y as f64);
    let (bx, by) = (b.x as f64, b.y as f64);
    let (cx, cy) = (c.x as f64, c.y as f64);
    let det = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy);
    if det.abs() < 1e-12 {
        return None;
    }
    let l1 = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / det;
    let l2 = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / det;
    let l3 = 1.0 - l1 - l2;
    Some([l1 as f32, l2 as f32, l3 as f32])
}

/// Signed area ×2 of a triangle (positive when counter-clockwise in a y-up
/// frame, i.e. clockwise on screen).
pub fn orient(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    (b.x as f64 - a.x as f64) * (c.y as f64 - a.y as f64)
        - (b.y as f64 - a.y as f64) * (c.x as f64 - a.x as f64)
}

/// The closest point to `p` on segment `a`–`b`, and its parameter along it.
pub fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> (Vec2, f32) {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 1e-12 {
        return (a, 0.0);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (a + ab * t, t)
}

/// Where a point sits relative to a triangle mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Location {
    /// Inside triangle `index`, with barycentric weights for its corners.
    Inside {
        /// Triangle index.
        triangle: usize,
        /// Weights for the triangle's three vertices.
        weights: [f32; 3],
    },
    /// Outside every triangle; the weights project onto the nearest edge of
    /// triangle `triangle` so data can still be extrapolated sensibly.
    Outside {
        /// Triangle owning the nearest edge.
        triangle: usize,
        /// Weights for the triangle's three vertices (one is zero).
        weights: [f32; 3],
        /// Distance to that edge.
        distance: f32,
    },
}

impl Location {
    /// The triangle and weights, whichever case applies.
    pub fn parts(&self) -> (usize, [f32; 3]) {
        match *self {
            Location::Inside { triangle, weights }
            | Location::Outside {
                triangle, weights, ..
            } => (triangle, weights),
        }
    }
}

/// Find where `p` lies in a mesh.
pub fn locate(p: Vec2, points: &[Vec2], triangles: &[[u32; 3]]) -> Option<Location> {
    let mut nearest: Option<Location> = None;
    let mut best = f32::INFINITY;
    for (t, tri) in triangles.iter().enumerate() {
        let Some(corners) = corners(points, tri) else {
            continue;
        };
        if let Some(w) = barycentric(p, corners[0], corners[1], corners[2]) {
            if w.iter().all(|&x| x >= -1e-5) {
                return Some(Location::Inside {
                    triangle: t,
                    weights: w,
                });
            }
        }
        for e in 0..3 {
            let (a, b) = (corners[e], corners[(e + 1) % 3]);
            let (q, s) = closest_on_segment(p, a, b);
            let d = q.distance(p);
            if d < best {
                best = d;
                let mut weights = [0.0; 3];
                weights[e] = 1.0 - s;
                weights[(e + 1) % 3] = s;
                nearest = Some(Location::Outside {
                    triangle: t,
                    weights,
                    distance: d,
                });
            }
        }
    }
    nearest
}

fn corners(points: &[Vec2], tri: &[u32; 3]) -> Option<[Vec2; 3]> {
    Some([
        *points.get(tri[0] as usize)?,
        *points.get(tri[1] as usize)?,
        *points.get(tri[2] as usize)?,
    ])
}

/// Delaunay triangulation of a point set (Bowyer–Watson).
///
/// Duplicate points (closer than a hundredth of a pixel) are merged into the
/// first occurrence; the returned triangles index into `points` and are wound
/// consistently. Collinear or fewer than three distinct points produce no
/// triangles.
pub fn delaunay(points: &[Vec2]) -> Vec<[u32; 3]> {
    // De-duplicate, remembering which input index each unique point came from.
    let mut unique: Vec<(f64, f64, u32)> = Vec::with_capacity(points.len());
    let mut seen: HashSet<(i64, i64)> = HashSet::new();
    for (i, p) in points.iter().enumerate() {
        if !p.is_finite() {
            continue;
        }
        let key = (
            (p.x as f64 * 100.0).round() as i64,
            (p.y as f64 * 100.0).round() as i64,
        );
        if seen.insert(key) {
            unique.push((p.x as f64, p.y as f64, i as u32));
        }
    }
    if unique.len() < 3 {
        return Vec::new();
    }
    // Insert in a spatially coherent order: sort by x then y.
    unique.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y, _) in &unique {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let span = (max_x - min_x).max(max_y - min_y).max(1.0);
    let (mx, my) = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);

    // Working vertex list: the unique points followed by a super-triangle.
    let mut verts: Vec<(f64, f64)> = unique.iter().map(|&(x, y, _)| (x, y)).collect();
    let n = verts.len();
    verts.push((mx - 20.0 * span, my - span));
    verts.push((mx, my + 20.0 * span));
    verts.push((mx + 20.0 * span, my - span));

    struct Tri {
        v: [usize; 3],
        cx: f64,
        cy: f64,
        r2: f64,
    }
    let make = |verts: &[(f64, f64)], v: [usize; 3]| -> Option<Tri> {
        let (ax, ay) = verts[v[0]];
        let (bx, by) = verts[v[1]];
        let (cx, cy) = verts[v[2]];
        let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
        if d.abs() < 1e-18 {
            return None;
        }
        let a2 = ax * ax + ay * ay;
        let b2 = bx * bx + by * by;
        let c2 = cx * cx + cy * cy;
        let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
        let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
        let r2 = (ax - ux).powi(2) + (ay - uy).powi(2);
        Some(Tri {
            v,
            cx: ux,
            cy: uy,
            r2,
        })
    };

    let mut tris: Vec<Tri> = Vec::new();
    if let Some(t) = make(&verts, [n, n + 1, n + 2]) {
        tris.push(t);
    }
    for i in 0..n {
        let (px, py) = verts[i];
        let mut bad: Vec<usize> = Vec::new();
        for (t, tri) in tris.iter().enumerate() {
            let d2 = (px - tri.cx).powi(2) + (py - tri.cy).powi(2);
            // A small relative tolerance keeps co-circular grids stable.
            if d2 < tri.r2 * (1.0 - 1e-12) {
                bad.push(t);
            }
        }
        if bad.is_empty() {
            continue;
        }
        // Boundary of the cavity: edges used by exactly one bad triangle.
        // A BTreeMap keeps the output order deterministic run to run.
        let mut edge_count: BTreeMap<(usize, usize), (usize, (usize, usize))> = BTreeMap::new();
        for &t in &bad {
            let v = tris[t].v;
            for e in 0..3 {
                let a = v[e];
                let b = v[(e + 1) % 3];
                let key = (a.min(b), a.max(b));
                edge_count
                    .entry(key)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, (a, b)));
            }
        }
        // Remove bad triangles (highest index first keeps indices valid).
        bad.sort_unstable_by(|a, b| b.cmp(a));
        for t in bad {
            tris.swap_remove(t);
        }
        for (_, (count, (a, b))) in edge_count {
            if count == 1 {
                if let Some(t) = make(&verts, [a, b, i]) {
                    tris.push(t);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(tris.len());
    for tri in tris {
        if tri.v.iter().any(|&v| v >= n) {
            continue;
        }
        let [a, b, c] = tri.v;
        let (ax, ay) = verts[a];
        let (bx, by) = verts[b];
        let (cx, cy) = verts[c];
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < 1e-9 {
            continue;
        }
        let map = |v: usize| unique[v].2;
        if area > 0.0 {
            out.push([map(a), map(b), map(c)]);
        } else {
            out.push([map(a), map(c), map(b)]);
        }
    }
    out
}

/// Triangulate a simple polygon (given as vertex indices in order) by ear
/// clipping. Used to fill the hole left when a vertex is deleted.
pub fn ear_clip(points: &[Vec2], polygon: &[u32]) -> Vec<[u32; 3]> {
    let mut ring: Vec<u32> = polygon
        .iter()
        .copied()
        .filter(|&i| (i as usize) < points.len())
        .collect();
    let mut out = Vec::new();
    if ring.len() < 3 {
        return out;
    }
    let area: f64 = (0..ring.len())
        .map(|i| {
            let a = points[ring[i] as usize];
            let b = points[ring[(i + 1) % ring.len()] as usize];
            a.x as f64 * b.y as f64 - b.x as f64 * a.y as f64
        })
        .sum();
    let ccw = area > 0.0;
    let mut guard = 0;
    while ring.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = ring.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
            let (a, b, c) = (points[ia as usize], points[ib as usize], points[ic as usize]);
            let o = orient(a, b, c);
            let convex = if ccw { o > 1e-9 } else { o < -1e-9 };
            if !convex {
                continue;
            }
            // Any other ring vertex inside the ear *or on its edges* blocks
            // it: a reflex vertex touching the diagonal means the ear would
            // cover area outside the polygon.
            let blocked = ring.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = points[j as usize];
                if p.distance(a) < 1e-6 || p.distance(b) < 1e-6 || p.distance(c) < 1e-6 {
                    return false;
                }
                barycentric(p, a, b, c)
                    .map(|w| w.iter().all(|&x| x >= -1e-6))
                    .unwrap_or(false)
            });
            if blocked {
                continue;
            }
            out.push([ia, ib, ic]);
            ring.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // Degenerate remainder: fan it rather than looping forever.
            break;
        }
    }
    if ring.len() >= 3 {
        for i in 1..ring.len() - 1 {
            let (a, b, c) = (ring[0], ring[i], ring[i + 1]);
            if orient(points[a as usize], points[b as usize], points[c as usize]).abs() > 1e-9 {
                out.push([a, b, c]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::math::vec2;

    fn total_area(points: &[Vec2], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| {
                orient(
                    points[t[0] as usize],
                    points[t[1] as usize],
                    points[t[2] as usize],
                )
                .abs()
                    * 0.5
            })
            .sum()
    }

    #[test]
    fn barycentric_weights_reconstruct_the_point() {
        let (a, b, c) = (vec2(0.0, 0.0), vec2(10.0, 0.0), vec2(0.0, 10.0));
        let w = barycentric(vec2(2.0, 3.0), a, b, c).expect("non-degenerate");
        let p = a * w[0] + b * w[1] + c * w[2];
        assert!((p.x - 2.0).abs() < 1e-5 && (p.y - 3.0).abs() < 1e-5);
        assert!(barycentric(vec2(1.0, 1.0), a, a, a).is_none());
    }

    #[test]
    fn a_square_grid_triangulates_to_its_full_area() {
        let mut points = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                points.push(vec2(x as f32 * 10.0, y as f32 * 10.0));
            }
        }
        let tris = delaunay(&points);
        assert_eq!(tris.len(), 32, "a 4×4 cell grid has 32 triangles");
        assert!((total_area(&points, &tris) - 1600.0).abs() < 1e-3);
    }

    #[test]
    fn random_points_give_a_valid_delaunay_triangulation() {
        let mut seed = 12345u32;
        let mut rand = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 100.0
        };
        let points: Vec<Vec2> = (0..200).map(|_| vec2(rand(), rand())).collect();
        let tris = delaunay(&points);
        assert!(!tris.is_empty());
        // Empty-circumcircle property.
        for t in &tris {
            let (a, b, c) = (
                points[t[0] as usize],
                points[t[1] as usize],
                points[t[2] as usize],
            );
            let (ax, ay, bx, by, cx, cy) = (
                a.x as f64, a.y as f64, b.x as f64, b.y as f64, c.x as f64, c.y as f64,
            );
            let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
            let ux = ((ax * ax + ay * ay) * (by - cy)
                + (bx * bx + by * by) * (cy - ay)
                + (cx * cx + cy * cy) * (ay - by))
                / d;
            let uy = ((ax * ax + ay * ay) * (cx - bx)
                + (bx * bx + by * by) * (ax - cx)
                + (cx * cx + cy * cy) * (bx - ax))
                / d;
            let r2 = (ax - ux).powi(2) + (ay - uy).powi(2);
            for (i, p) in points.iter().enumerate() {
                if t.contains(&(i as u32)) {
                    continue;
                }
                let d2 = (p.x as f64 - ux).powi(2) + (p.y as f64 - uy).powi(2);
                assert!(d2 >= r2 * (1.0 - 1e-6), "point {i} inside a circumcircle");
            }
        }
    }

    #[test]
    fn duplicates_and_collinear_points_are_handled() {
        let line = vec![vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(2.0, 0.0)];
        assert!(delaunay(&line).is_empty());
        let dup = vec![vec2(0.0, 0.0), vec2(0.0, 0.0), vec2(4.0, 0.0), vec2(0.0, 4.0)];
        let tris = delaunay(&dup);
        assert_eq!(tris.len(), 1);
        assert!(
            !tris[0].contains(&1),
            "the duplicate maps to its first occurrence"
        );
    }

    #[test]
    fn locate_finds_inside_and_nearest_outside() {
        let points = vec![vec2(0.0, 0.0), vec2(10.0, 0.0), vec2(0.0, 10.0)];
        let tris = vec![[0, 1, 2]];
        match locate(vec2(1.0, 1.0), &points, &tris) {
            Some(Location::Inside { triangle: 0, .. }) => {}
            other => panic!("expected inside, got {other:?}"),
        }
        match locate(vec2(5.0, -3.0), &points, &tris) {
            Some(Location::Outside {
                distance, weights, ..
            }) => {
                assert!((distance - 3.0).abs() < 1e-5);
                assert!((weights[0] - 0.5).abs() < 1e-5 && (weights[1] - 0.5).abs() < 1e-5);
            }
            other => panic!("expected outside, got {other:?}"),
        }
    }

    #[test]
    fn ear_clipping_fills_a_concave_polygon() {
        // An L shape.
        let points = vec![
            vec2(0.0, 0.0),
            vec2(20.0, 0.0),
            vec2(20.0, 10.0),
            vec2(10.0, 10.0),
            vec2(10.0, 20.0),
            vec2(0.0, 20.0),
        ];
        let tris = ear_clip(&points, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(tris.len(), 4);
        assert!((total_area(&points, &tris) - 300.0).abs() < 1e-3);
    }
}
