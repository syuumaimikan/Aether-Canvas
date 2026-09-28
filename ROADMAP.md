# Roadmap

Phases ship complete: a phase is done when its features work, are tested, and
are reachable from the UI. Nothing lands as a disabled button or a "coming
soon" panel.

## Phase 0 — Foundation ✅

- Cargo workspace, layered crates, error types, logging
- GPU-backed window (wgpu) with a dockable, tabbed, splittable panel layout
- Document model, canvas, project system
- Zoom, pan, rotate, mirror, checkerboard

## Phase 1 — Raster MVP ✅

- Raster layers, layer panel, add/delete/duplicate/reorder/merge
- Brush engine, eraser, bucket fill, eyedropper, move tool
- Colour picker, palettes, recent colours
- Undo/redo with a history panel
- `.aether` save/load, PNG (and JPEG/WebP/TIFF/BMP/GIF) export
- Three workspaces, three themes, English and Japanese, rebindable shortcuts

## Phase 2 — Advanced painting ✅

- 23 blend modes; groups (isolated and pass-through); layer masks; clipping
  layers; transparency lock
- Rectangle, ellipse, lasso and wand selections with add / subtract / intersect,
  feather, expand and contract
- Adjustment layers with an interactive curve editor: levels, curves,
  hue/saturation, brightness/contrast, exposure, gamma, colour balance, invert,
  threshold, posterize, grayscale
- Non-destructive per-layer effect stack: blur, motion blur, sharpen, glow, drop
  shadow, outline, colour overlay, grain, and any adjustment
- Destructive Filter menu sharing the same kernels
- Interactive transform tool: move, scale, rotate, skew, perspective distort and
  mesh warp, with modal apply/cancel
- Liquify brush: push, twirl, pinch, bloat, restore
- Textured brushes (procedural or image grain) and image-stamp pattern tips
- Brush presets saved and shared as plain JSON
- Pressure from the device where the platform reports it, with a speed-derived
  fallback and an off switch

Carried forward: desktop tablet APIs that `winit` does not expose yet (Wintab on
Windows, and Windows Ink) still fall back to the speed mapping; dual-tip brushes
and non-destructive *filter* layers (as opposed to per-layer effects) are not
implemented.

## Phase 3 — Pixel art

- Pixel-perfect line and freehand drawing, dither and shade tools
- Indexed colour mode, palette lock, palette swap, colour ramps
- Tile sets, tile maps, auto-tiling, collision metadata
- Frame animation, onion skin, frame tags, sprite-sheet export

## Phase 4 — Vector

- Path, Bézier and shape primitives with a node editor
- Stroke, fill, gradients
- Boolean operations (union, subtract, intersect, exclude, divide)
- Non-destructive path modifiers (offset, outline, repeat, mirror, roughen)
- Text engine: variable fonts, kerning, tracking, text on a path, vertical text
- SVG import and export

## Phase 5 — Animation ✅ (parameter animation)

- Timeline with transport, keyframe rows (select, retime, key by
  double-click), per-key easing and a curve preview
- Step, linear, cubic Bézier, ease in/out/in-out, back, elastic, bounce and
  spring interpolation
- Layered animator with override/additive layers and crossfades; expressions
- Auto-blink, breathing, look-at and lip sync (live, or baked from WAV)
- Sandboxed expression drivers
- Export as GIF, PNG sequence and sprite sheet + JSON atlas, with physics

Carried forward: onion skinning and frame-by-frame (raster) animation, which
arrive with the pixel-art frame model of Phase 3; a draggable Bézier-handle
graph editor (keys take numeric handles today); video (MP4) export, which
arrives with the FFmpeg pipeline of Phase 8.

## Phase 6 — Rigging ✅

- User-defined parameters driving N-dimensional keyform grids (linear or
  smooth), cyclic parameters and additive blend shapes
- Meshes bound to raster layers: auto-mesh with a coverage guarantee, manual
  editing, re-meshing that keeps keyforms; opacity, tint and draw-order keys;
  glue; jiggle
- Warp (bilinear/bicubic) and rotation deformers, nested; drags mapped
  through deformed parents
- Bones, FK, two-bone and CCD inverse kinematics, linear blend skinning,
  automatic weights
- Fixed-step pendulum physics with wind, stiffness, angle limits, colliders
- Generators: 3D head turn, sway, close, keyform mirroring, standard physics
- One-click auto-rig from English/Japanese layer names
- Layered PSD import and export
- Runtime: an open runtime-model format (JSON + texture atlases), exported
  from the editor or headlessly (`--auto-rig --export-model`); `aether-player`
  running the editor's rig code, with a C ABI, a WebAssembly + WebGL web
  player and a software renderer, all parity-tested against the editor
- Face tracking: one mapping from ARKit/MediaPipe-style trackers onto the
  standard parameters, and webcam tracking in the web player
- Live2D interchange: motions (`.motion3.json`) and expressions
  (`.exp3.json`), both ways

Carried forward: path deformers, weight painting with a brush (weights are
automatic and per-vertex editable through the data), a formal pose-group
(part switching) editor, and ready-made engine packages (a Unity package and
a Godot extension over the C ABI).

## Phase 7 — Motion graphics

- Compositions and precomposition, nested compositions
- Camera, null objects, parenting and transform inheritance
- Text animators, motion blur
- A sandboxed expression language (`sin(time * 2)`, `wiggle(2, 20)`)

## Phase 8 — Compositing and VFX

- Node graph that shares the layer model rather than duplicating it
- GPU render graph with pass-level caching
- Effect stacks: blur, glow, distortion, chromatic aberration, vignette, grain
- GPU particle system
- Video import/export through an FFmpeg-backed pipeline

## Phase 9 — Plugins

- Sandboxed plugin loader and a stable API
- Plugin-defined brushes, tools, effects, nodes, importers and exporters
- Optional, always-explicit AI assists (selection, line-art cleanup, upscale,
  rig assist) that never modify artwork without being asked

## Phase 10 — Performance and resilience

- GPU profiling, memory budgets, sparse tile storage for very large canvases
- Render-thread separation and a background worker pool
- Autosave, crash recovery and version history
