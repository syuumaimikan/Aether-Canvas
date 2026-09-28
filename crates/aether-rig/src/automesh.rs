//! Automatic meshing from a layer's alpha.
//!
//! Meshing by hand is the most tedious part of rigging, so the default is to
//! let the machine do it and let the artist adjust. The algorithm:
//!
//! 1. Threshold the layer's alpha and **dilate** it by a margin, so the mesh
//!    boundary sits outside the painted edge and never cuts anti-aliasing.
//! 2. Place **boundary points** on the dilated outline at a fine spacing, and
//!    **interior points** on a staggered grid at a coarser spacing, keeping
//!    them away from the outline.
//! 3. **Delaunay-triangulate** all points and discard triangles whose centroid
//!    falls outside the dilated shape, which carves out concavities and holes.
//!
//! The result is checked in tests to cover every opaque pixel of the layer.

use crate::geom;
use aether_core::math::{IRect, Vec2};
use aether_raster::Pixmap;
use serde::{Deserialize, Serialize};

/// Knobs for [`auto_mesh`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoMeshOptions {
    /// Distance between interior points, pixels.
    pub spacing: f32,
    /// Distance between points along the outline, pixels.
    pub edge_spacing: f32,
    /// How far the outline sits outside the painted pixels.
    pub margin: f32,
    /// Alpha at or below this counts as empty.
    pub alpha_threshold: u8,
}

impl Default for AutoMeshOptions {
    fn default() -> Self {
        Self {
            spacing: 32.0,
            edge_spacing: 20.0,
            margin: 6.0,
            alpha_threshold: 8,
        }
    }
}

impl AutoMeshOptions {
    /// Options scaled so a shape of the given bounds gets a few hundred
    /// vertices at `density` 1 (higher density, more vertices).
    pub fn for_bounds(bounds: IRect, density: f32) -> Self {
        let size = (bounds.width.max(bounds.height)).max(1) as f32;
        let density = density.clamp(0.1, 8.0);
        let spacing = (size / (14.0 * density)).clamp(4.0, 256.0);
        Self {
            spacing,
            edge_spacing: (spacing * 0.6).max(3.0),
            margin: (spacing * 0.2).clamp(2.0, 16.0),
            alpha_threshold: 8,
        }
    }
}

/// A generated mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedMesh {
    /// Vertex positions, document space.
    pub vertices: Vec<Vec2>,
    /// Triangles.
    pub triangles: Vec<[u32; 3]>,
}

/// Mesh the opaque region of `pixmap`, or `None` when it is empty.
pub fn auto_mesh(pixmap: &Pixmap, options: &AutoMeshOptions) -> Option<GeneratedMesh> {
    let opaque = opaque_bounds(pixmap, options.alpha_threshold)?;
    let margin = options.margin.max(0.0).ceil() as i32 + 1;
    let area = opaque.expanded(margin + 1);
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return None;
    }

    // Distance from each pixel to the nearest opaque pixel (chamfer 3-4, in
    // units of 1/3 px), then threshold at the margin.
    const INF: u32 = u32::MAX / 4;
    let mut dist = vec![INF; w * h];
    for y in 0..h {
        for x in 0..w {
            let px = area.x + x as i32;
            let py = area.y + y as i32;
            if pixmap.bounds().contains(px, py) && pixmap.get(px, py).a > options.alpha_threshold {
                dist[y * w + x] = 0;
            }
        }
    }
    chamfer(&mut dist, w, h);
    let limit = (options.margin.max(0.0) * 3.0).round() as u32;
    let inside = |x: i64, y: i64| -> bool {
        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && dist[y as usize * w + x as usize] <= limit
    };

    // Boundary points: outline pixels, thinned to the edge spacing. Every
    // outline point is remembered for the coverage repair pass below.
    let edge_spacing = options.edge_spacing.max(2.0);
    let mut points: Vec<Vec2> = Vec::new();
    let mut outline: Vec<Vec2> = Vec::new();
    let mut grid = SpatialGrid::new(edge_spacing);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !inside(x, y) {
                continue;
            }
            let on_edge = !inside(x - 1, y) || !inside(x + 1, y) || !inside(x, y - 1) || !inside(x, y + 1);
            if !on_edge {
                continue;
            }
            // The outer corner of the edge pixel, so the mesh encloses it.
            let mut p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            if !inside(x - 1, y) {
                p.x -= 0.5;
            }
            if !inside(x + 1, y) {
                p.x += 0.5;
            }
            if !inside(x, y - 1) {
                p.y -= 0.5;
            }
            if !inside(x, y + 1) {
                p.y += 0.5;
            }
            outline.push(p);
            if grid.is_clear(p, edge_spacing) {
                grid.insert(p);
                points.push(p);
            }
        }
    }
    let mut boundary_count = points.len();

    // Interior points on a staggered grid, away from the outline.
    let spacing = options.spacing.max(edge_spacing);
    let row_height = spacing * 0.866;
    let rows = (h as f32 / row_height).ceil() as usize + 1;
    let cols = (w as f32 / spacing).ceil() as usize + 1;
    for r in 0..rows {
        let y = r as f32 * row_height + row_height * 0.5;
        let offset = if r % 2 == 0 { 0.0 } else { spacing * 0.5 };
        for c in 0..cols {
            let x = c as f32 * spacing + offset + spacing * 0.25;
            let (ix, iy) = (x as i64, y as i64);
            if !inside(ix, iy) {
                continue;
            }
            // Keep interior points clear of the outline points (and of each
            // other) so no sliver triangles form along the edge.
            let p = Vec2::new(x, y);
            if !grid.is_clear(p, spacing * 0.45) {
                continue;
            }
            grid.insert(p);
            points.push(p);
        }
    }
    if points.len() < 3 {
        return None;
    }

    let triangulate = |points: &[Vec2], boundary_count: usize| -> Vec<[u32; 3]> {
        geom::delaunay(points)
            .into_iter()
            .filter(|t| {
                let (a, b, c) = (
                    points[t[0] as usize],
                    points[t[1] as usize],
                    points[t[2] as usize],
                );
                let centroid = (a + b + c) / 3.0;
                // Only boundary-to-boundary triangles can bridge a concavity; check
                // them more strictly with their edge midpoints too.
                let all_boundary = t.iter().all(|&i| (i as usize) < boundary_count);
                let probe = |p: Vec2| inside(p.x.floor() as i64, p.y.floor() as i64);
                if !probe(centroid) {
                    return false;
                }
                if all_boundary {
                    let mids = [(a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5];
                    let outside = mids.iter().filter(|m| !probe(**m)).count();
                    // A triangle hugging a convex outline has at most one edge
                    // along it; one spanning a gap has several edges outside.
                    return outside <= 1;
                }
                true
            })
            .collect()
    };

    // The boundary-only check relies on outline points coming first, so the
    // repair pass inserts new outline points before the interior ones.
    let mut triangles = triangulate(&points, boundary_count);

    // Coverage repair: a chord between two outline points can shave a sharp
    // corner. Any painted pixel left uncovered pulls the nearest outline
    // point into the mesh, and the mesh is rebuilt.
    for _ in 0..6 {
        let coverage = aether_raster::mesh_coverage(w as u32, h as u32, &points, &triangles);
        let mut missing: Vec<Vec2> = Vec::new();
        let mut seen = SpatialGrid::new(edge_spacing * 0.5);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if dist[y as usize * w + x as usize] != 0 || coverage.get(x, y) != 0 {
                    continue;
                }
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if seen.is_clear(p, edge_spacing * 0.5) {
                    seen.insert(p);
                    missing.push(p);
                }
            }
        }
        if missing.is_empty() {
            break;
        }
        for m in missing {
            let nearest = outline
                .iter()
                .copied()
                .min_by(|a, b| a.distance(m).total_cmp(&b.distance(m)));
            if let Some(q) = nearest {
                if !points.iter().any(|p| p.distance(q) < 0.25) {
                    points.insert(boundary_count, q);
                    boundary_count += 1;
                }
            }
        }
        triangles = triangulate(&points, boundary_count);
    }

    // Compact: drop unused points and shift into document space.
    let mut remap = vec![u32::MAX; points.len()];
    let mut vertices = Vec::new();
    let mut out_tris = Vec::with_capacity(triangles.len());
    for t in &triangles {
        let mut mapped = [0u32; 3];
        for (k, &i) in t.iter().enumerate() {
            if remap[i as usize] == u32::MAX {
                remap[i as usize] = vertices.len() as u32;
                vertices.push(points[i as usize] + Vec2::new(area.x as f32, area.y as f32));
            }
            mapped[k] = remap[i as usize];
        }
        out_tris.push(mapped);
    }
    if out_tris.is_empty() {
        return None;
    }
    Some(GeneratedMesh {
        vertices,
        triangles: out_tris,
    })
}

/// Bounds of the pixels with alpha above `threshold`.
pub fn opaque_bounds(pixmap: &Pixmap, threshold: u8) -> Option<IRect> {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..h {
        let row = pixmap.row(y as u32);
        for x in 0..w {
            if row[x as usize * 4 + 3] > threshold {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x1 >= x0).then(|| IRect::from_bounds(x0, y0, x1 + 1, y1 + 1))
}

/// Two-pass 3-4 chamfer distance transform in place.
fn chamfer(dist: &mut [u32], w: usize, h: usize) {
    let at = |x: usize, y: usize| y * w + x;
    for y in 0..h {
        for x in 0..w {
            let mut d = dist[at(x, y)];
            if x > 0 {
                d = d.min(dist[at(x - 1, y)] + 3);
            }
            if y > 0 {
                d = d.min(dist[at(x, y - 1)] + 3);
                if x > 0 {
                    d = d.min(dist[at(x - 1, y - 1)] + 4);
                }
                if x + 1 < w {
                    d = d.min(dist[at(x + 1, y - 1)] + 4);
                }
            }
            dist[at(x, y)] = d;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let mut d = dist[at(x, y)];
            if x + 1 < w {
                d = d.min(dist[at(x + 1, y)] + 3);
            }
            if y + 1 < h {
                d = d.min(dist[at(x, y + 1)] + 3);
                if x + 1 < w {
                    d = d.min(dist[at(x + 1, y + 1)] + 4);
                }
                if x > 0 {
                    d = d.min(dist[at(x - 1, y + 1)] + 4);
                }
            }
            dist[at(x, y)] = d;
        }
    }
}

/// A hash grid for "is there already a point within r?" queries.
struct SpatialGrid {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32), Vec<Vec2>>,
}

impl SpatialGrid {
    fn new(cell: f32) -> Self {
        Self {
            cell: cell.max(1.0),
            cells: std::collections::HashMap::new(),
        }
    }

    fn key(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    fn insert(&mut self, p: Vec2) {
        let key = self.key(p);
        self.cells.entry(key).or_default().push(p);
    }

    fn is_clear(&self, p: Vec2, radius: f32) -> bool {
        let (kx, ky) = self.key(p);
        let reach = (radius / self.cell).ceil() as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                if let Some(points) = self.cells.get(&(kx + dx, ky + dy)) {
                    if points.iter().any(|q| q.distance(p) < radius) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_core::color::Rgba8;
    use aether_raster::mesh_coverage;

    fn disc(size: u32, cx: f32, cy: f32, r: f32) -> Pixmap {
        let mut pm = Pixmap::new(size, size);
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                if d < r {
                    pm.set(x, y, Rgba8::new(200, 100, 50, 255));
                } else if d < r + 1.0 {
                    pm.set(x, y, Rgba8::new(200, 100, 50, ((r + 1.0 - d) * 255.0) as u8));
                }
            }
        }
        pm
    }

    fn uncovered(pixmap: &Pixmap, mesh: &GeneratedMesh) -> usize {
        let coverage = mesh_coverage(pixmap.width(), pixmap.height(), &mesh.vertices, &mesh.triangles);
        let mut missing = 0;
        for y in 0..pixmap.height() as i32 {
            for x in 0..pixmap.width() as i32 {
                if pixmap.get(x, y).a > 0 && coverage.get(x, y) == 0 {
                    missing += 1;
                }
            }
        }
        missing
    }

    #[test]
    fn an_empty_layer_has_no_mesh() {
        assert!(auto_mesh(&Pixmap::new(32, 32), &AutoMeshOptions::default()).is_none());
    }

    #[test]
    fn a_disc_is_fully_covered_by_a_reasonable_mesh() {
        let pm = disc(200, 100.0, 100.0, 60.0);
        let options = AutoMeshOptions::for_bounds(IRect::new(0, 0, 120, 120), 1.0);
        let mesh = auto_mesh(&pm, &options).expect("mesh");
        assert_eq!(uncovered(&pm, &mesh), 0, "every painted pixel is inside the mesh");
        assert!(
            (20..600).contains(&mesh.vertices.len()),
            "{} vertices",
            mesh.vertices.len()
        );
        // The mesh hugs the shape: it does not cover the far corners.
        let coverage = mesh_coverage(200, 200, &mesh.vertices, &mesh.triangles);
        assert_eq!(coverage.get(5, 5), 0);
    }

    #[test]
    fn concave_shapes_are_carved_out() {
        // Two blobs joined by a thin bar: a U shape.
        let mut pm = Pixmap::new(200, 120);
        for y in 0..120 {
            for x in 0..200 {
                let left = (20..60).contains(&x) && (10..110).contains(&y);
                let right = (140..180).contains(&x) && (10..110).contains(&y);
                let bar = (20..180).contains(&x) && (90..110).contains(&y);
                if left || right || bar {
                    pm.set(x, y, Rgba8::WHITE);
                }
            }
        }
        let options = AutoMeshOptions {
            spacing: 16.0,
            edge_spacing: 10.0,
            margin: 3.0,
            alpha_threshold: 8,
        };
        let mesh = auto_mesh(&pm, &options).expect("mesh");
        assert_eq!(uncovered(&pm, &mesh), 0);
        let coverage = mesh_coverage(200, 120, &mesh.vertices, &mesh.triangles);
        assert_eq!(coverage.get(100, 40), 0, "the gap inside the U stays empty");
    }

    #[test]
    fn density_controls_vertex_count() {
        let pm = disc(200, 100.0, 100.0, 70.0);
        let bounds = IRect::new(30, 30, 140, 140);
        let coarse = auto_mesh(&pm, &AutoMeshOptions::for_bounds(bounds, 0.5)).expect("mesh");
        let fine = auto_mesh(&pm, &AutoMeshOptions::for_bounds(bounds, 2.0)).expect("mesh");
        assert!(fine.vertices.len() > coarse.vertices.len() * 2);
    }
}
