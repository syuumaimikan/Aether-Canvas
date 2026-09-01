# Aether Canvas

An integrated 2D creative environment written in Rust: painting, pixel art,
vector work, 2D rigging, animation, motion graphics and compositing in one
project, one document model and one timeline.

> **Status: Phase 1 complete.** The application builds, runs, and is usable for
> raster painting: layers, blend modes, masks, selections, a real brush engine,
> full undo/redo, an open project format and image export. The later phases
> (vector, pixel-art tooling, rigging, animation, VFX) are designed for but not
> yet implemented — see [ROADMAP.md](ROADMAP.md) for exactly what exists today.
> Nothing in this repository is a mock: if the UI offers it, it works.

## What works today

**Document and layers**
- Raster, group, adjustment, fill and plugin-defined layer types in one tree
- Nested groups, isolated and pass-through blending
- 23 blend modes following the W3C compositing spec
- Per-layer opacity, visibility, lock, transparency lock, clipping and masks
- Non-destructive adjustment layers (levels, curves, hue/saturation, and more)

**Painting**
- Dab-based brush engine with spacing, hardness, flow/opacity separation,
  scatter, jitter, elliptical tips and a stroke stabiliser
- Pressure, tilt and speed input mapping (mouse input synthesises speed)
- Six built-in presets; every parameter is editable and savable
- Eraser, bucket fill, eyedropper, move, and rectangle/ellipse/lasso/wand
  selection tools

**Canvas**
- GPU-accelerated view (wgpu) with pan, zoom, rotation, mirror and a pixel grid
- Tile-based incremental compositing: a brush dab re-composites and re-uploads
  only the tiles it touched

**Editing**
- Every operation is an undoable command, with slider-drag coalescing,
  transactions and a clickable history panel

**Files**
- `.aether` project format: a plain ZIP of JSON + PNG, versioned and migratable
- Import and export PNG, JPEG, WebP, TIFF, BMP and GIF

**UI**
- Dockable, splittable, tabbed panels (drag them anywhere)
- Three workspaces (Illustration, Pixel Art, Compositing)
- Dark, light and high-contrast themes
- English and Japanese, with no UI strings hard-coded in widget code
- Fully rebindable keyboard shortcuts with conflict detection

## Building and running

Requires a recent stable Rust toolchain (1.82+).

```sh
cargo run --release -p aether-desktop            # start the editor
cargo run --release -p aether-desktop -- art.aether   # open a file
cargo test --workspace                           # run the test suite
```

On Linux the window needs the usual desktop libraries at runtime
(`libxkbcommon`, plus X11 or Wayland client libraries) and a Vulkan- or
GL-capable driver.

### Without a display

The whole imaging pipeline is independent of the window system, so it can be
driven headlessly:

```sh
cargo run -p aether-desktop --example headless_render -- out.png
```

This builds a document in code, paints with the brush engine, composites and
writes both `out.png` and `out.aether`.

## Repository layout

```text
crates/
  aether-core/      math, colour, blend modes, ids, errors, input
  aether-raster/    pixel buffers, tiles, compositing kernels, brush engine
  aether-document/  layer tree, document model, commands, undo history
  aether-render/    compositor, render cache, viewport maths
  aether-io/        .aether project container, image import/export
  aether-ui/        panels, tools, docking layout, application shell
apps/desktop/       the binary, plus end-to-end tests and examples
docs/               architecture notes and the file-format specification
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — how the pieces fit together and why
- [ROADMAP.md](ROADMAP.md) — what is built, what is next
- [CONTRIBUTING.md](CONTRIBUTING.md) — conventions and expectations
- [docs/FILE_FORMAT.md](docs/FILE_FORMAT.md) — the `.aether` container

## Licence

Dual-licensed under MIT or Apache-2.0, at your option.
