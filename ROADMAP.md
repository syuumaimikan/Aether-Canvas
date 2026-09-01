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

## Phase 2 — Advanced painting 🚧 (partly done)

Done: 23 blend modes, groups (isolated and pass-through), layer masks, clipping
layers, transparency lock, rectangle/ellipse/lasso/wand selections with
add/subtract/intersect, feather/expand/contract, adjustment layers (levels,
curves, hue/saturation, brightness/contrast, exposure, gamma, colour balance,
invert, threshold, posterize, grayscale), gaussian blur and sharpen kernels.

Remaining:
- Platform tablet plumbing for real pressure, tilt and barrel rotation
- Textured, pattern and dual brushes; brush preset import/export
- Interactive transform tool (scale, rotate, skew, perspective, warp, liquify)
- Filter gallery in the UI, and filters as non-destructive layer effects

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

## Phase 5 — Animation

- Timeline, keyframes, dope sheet, graph editor
- Linear, stepped, Bézier, ease and spring interpolation
- Onion skinning across the timeline
- Frame and parameter animation sharing one model

## Phase 6 — Rigging

- Bones, forward and inverse kinematics, constraints
- Mesh deformers with vertex weights, warp and path deformers
- User-defined parameters driving keyforms (angle, eye open, mouth form, ...)
- Assisted auto-mesh, auto-weight and symmetry, all hand-editable

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
