# Architecture

This document explains the shape of Aether Canvas: the technology choices, the
crate layout, how data flows from a pointer event to a pixel on screen, and the
risks the design is trying to manage.

## 1. Technology choices

| Concern | Choice | Why |
| --- | --- | --- |
| Language | Rust (2021, stable) | Predictable performance without a GC, and fearless multithreading for a compositor that has to keep up with a stylus |
| GPU | `wgpu`, via `eframe` | One API over Vulkan, Metal, DX12 and WebGPU; the same code path can reach the browser later |
| Windowing | `winit`, via `eframe` | The de-facto standard, and what `wgpu` integrates with |
| UI | `egui` + `egui_dock` | Immediate-mode GPU UI with no WebView. Docking, tabs and floating panels come from `egui_dock`, and an immediate-mode tree is easy to keep in sync with a document that changes constantly |
| Images | `image` | Broad format coverage from one dependency, all pure Rust |
| Container | `zip` + `serde_json` | An open, inspectable project format |
| Parallelism | `rayon` | Data-parallel compositing without hand-rolling a thread pool |
| Logging | `tracing` | Structured, and ready for span-based profiling later |

Deliberate non-choices: no WebView (a painting app cannot afford a DOM in the
hot path), no retained-mode GUI toolkit (keeping widget state in sync with an
undoable document is a bug factory), and no bespoke maths crate dependency
(2D affine transforms are 150 lines and we want `serde` on them).

## 2. Crates

Dependencies point strictly downwards. Nothing below `aether-ui` knows a window
exists, and nothing below `aether-render` knows a GPU exists.

```text
aether-desktop  (binary: window, CLI, logging)
      |
  aether-ui     (panels, tools, docking, app shell)
      |
  aether-io     (.aether container, image/PSD import/export, model export)
      |                 \
aether-render           aether-player  (runtime: model format, player,
      |                   |             C ABI = WebAssembly interface,
aether-document           |             software renderer)
      |                   |
  aether-rig  <-----------'   (parameters, keyforms, deformers, bones,
      |                        physics, motions)
 aether-raster  (pixmaps, tiles, blending, brush engine, filters, meshes)
      |
  aether-core   (math, colour, blend modes, ids, errors, input)
```

`aether-player` sits beside the editor stack, not under it: it depends only
on the rig and raster crates, so it compiles to a small WebAssembly module
and a shared library with no document, UI or file-format code inside.

That ordering is what makes the project testable: 500+ of the tests run with no
window, no GPU and no filesystem.

## 3. Data model

```text
Project
 └── Document
      ├── Canvas (width, height, background, colour model)
      ├── LayerTree
      │    └── Layer { id, name, visible, locked, opacity, blend,
      │                transform, clipping, mask, content }
      │         └── LayerContent = Raster | Group | Adjustment | Fill | Custom
      ├── Selection
      ├── Rig  (parameters, meshes keyed by LayerId, deformers, bones,
      │         physics, drivers, motions, expressions, behaviours)
      ├── IdGenerator
      └── Metadata
```

Two decisions carry most of the weight:

**Common properties are separate from content.** Opacity, blending, masking,
clipping and locking behave the same for a painted layer, a group and a
plugin's layer, so the compositor implements them once. Adding a layer type
means adding a payload, not editing the compositor's bookkeeping.

**`LayerContent::Custom` is the plugin seam.** It carries a reverse-DNS type tag
and an opaque JSON payload. The compositor asks a registry whether anything can
render that tag; if not, the layer is skipped — but its data still round-trips
through save, load and undo. A project made with a plugin you do not have opens
without losing anything.

Ordering convention: **children are stored bottom-to-top**, because that is the
order the compositor wants. The layer panel shows the reverse.

## 4. Rendering

```text
Document
   │  dirty rectangle
   ▼
RenderCache ──▶ Compositor ──▶ Pixmap (document space)
                                  │  changed tiles only
                                  ▼
                            GPU texture ──▶ quad through the Viewport matrix
                                                      │
                                                      ▼
                                          checkerboard, artwork, selection,
                                          pixel grid, tool overlays
```

The compositor walks the tree bottom-to-top, keeping one backdrop buffer.
Four cases need more than a blend:

- **Clipping groups** — a run of clipping layers on a base layer is composited
  into a private buffer masked by the base's alpha, then blended in using the
  *base's* opacity and blend mode.
- **Isolated groups** — children composite onto an empty buffer so the group's
  blend mode applies to the group as a whole.
- **Pass-through groups** — children blend straight against the backdrop. A
  pass-through group that has its own opacity, mask or blend mode cannot behave
  that way, so it is isolated automatically.
- **Adjustment layers** — evaluated against the current backdrop and blended
  back in, which is what makes them non-destructive and maskable.

**Rigged layers.** The rig is posed once per pass. A raster layer whose mesh
is deformed is redrawn through the posed triangles (`aether_raster::mesh`) as
the layer's *content*, so effects, masks, clipping, blend modes and
adjustment layers above all see the deformed pixels. Keyed draw-order offsets
reorder siblings (carrying their clipping runs). A mesh at rest draws the
layer directly, so binding a mesh never changes a pixel until something
moves. Plain raster layers are borrowed rather than copied for each pass.

A layer's own **effect stack** runs between producing its content and blending
it in, so a drop shadow lands behind its layer but in front of everything below
it, and the layer's opacity and mask still apply to the finished result.

The CPU compositor is the reference implementation. Export and the canvas use
the same code, so what an artist sees is what gets saved. The GPU render graph
planned for Phase 8 will be validated against it.

### Tiling

Tiles are 256×256 and are the unit of three things: parallel work, texture
upload, and undo capture. A brush dab therefore re-composites a few hundred
pixels, uploads one tile, and copies one tile aside for undo — regardless of
canvas size.

## 5. Painting pipeline

```text
pointer event ─▶ viewport inverse ─▶ InputSample (document space)
                                          │
                                    StrokeState
                                    ├─ smoothing (stabiliser)
                                    ├─ spacing resampler
                                    └─ dynamics (pressure, speed)
                                          │  dabs
                                          ▼
                                    stroke buffer (Mask)
                                          │
              tile snapshot ─restore─▶ layer ─fill_masked─▶ pixels
                                          │
                     on release: RegionEdit(before, after) ─▶ History
```

The stroke buffer is the important part. Dabs accumulate coverage into a mask
rather than painting the layer directly, and the layer is rebuilt each frame
from the pre-stroke tiles plus the whole accumulated stroke. That is what makes
*flow* (build-up along the stroke) independent of *opacity* (a ceiling for the
stroke), and what stops a stroke that crosses itself from darkening at the
crossing.

## 5b. Transforming and deforming

Scale, rotate, skew, perspective distort and warp are one tool with one
representation — a destination quad — and one resampler: the homography that
maps the layer's content rectangle onto that quad
(`Perspective::from_quads`), optionally followed by a displacement field. A
"free" transform is simply a drag rule that keeps the quad a parallelogram, so
free and perspective modes cannot drift apart.

Liquify shares the displacement field. Both tools are *modal*: they keep the
original pixels aside, rebuild the layer from them on every gesture (which is
also what stops repeated passes from compounding resampling blur), and write a
single undo entry when the artist confirms.

## 5c. Rigging

```text
authored values ─▶ RigRuntime.tick ─────────────────────────────▶ Rig.dynamics
   (sliders)       timeline scrub / animator → expressions →        (values +
                   behaviours → drivers → physics → jiggle          jiggle offsets)
                                                                        │
Rig + values ─▶ Evaluator: skeleton FK → IK → deformer states ─▶ RigPose
                meshes: keyforms + blend shapes → skin → parents     │
                → dynamics → glue                                     ▼
                                                   compositor / overlay / export
```

Evaluation is a pure function of the rig and a set of parameter values: the
same values always give the same geometry, which is what lets the canvas,
export and any future runtime agree. Everything time-dependent lives in
`RigRuntime` and communicates only by producing parameter values and
per-vertex offsets. The editor diffs consecutive poses and marks only the
changed area dirty, so a blinking eye re-composites a few tiles.

Rest geometry is stored in document space; a vertex's rest position doubles as
its texture coordinate. Each deformer maps rest-space points, and nesting
composes the maps; editing tools invert the chain locally (a numerical
Jacobian) so drags land where the cursor is.

## 5d. Runtime

```text
Document ─▶ runtime_model::export ─▶ model.json + texture atlases
              (masks/effects baked,         │
               groups folded, atlas packed) ▼
                                      aether-player::Player
                          RigRuntime.tick ─▶ Rig::evaluate ─▶ draw list
                                                               │
                  ┌──────────────┬───────────────┬─────────────┤
                  ▼              ▼               ▼             ▼
            WebGL (JS)     your engine (C)   cpu::render    Rust hosts
            via WebAssembly
```

The player owns no rig logic of its own: it wraps the same `RigRuntime` and
`Rig::evaluate` the editor calls, then flattens the result into a draw list
(part, opacity, tint, blend, clipping mask) sorted exactly as the compositor
sorts layers. Renderers only draw triangles. The C ABI is the single foreign
interface — the WebAssembly module is that ABI compiled for `wasm32`, with no
imports — so the web player and native hosts cannot disagree. The software
renderer uses the compositor's own rasteriser and blend kernels, and the test
suite holds every renderer to it. See [docs/RUNTIME.md](docs/RUNTIME.md).

## 6. Undo

Every edit is a `Command` that can apply and reverse itself, stored in two
stacks. Commands keep the minimum state that makes both directions exact:

| Command | Stores |
| --- | --- |
| `RegionEdit` | before/after pixels of one rectangle |
| `AddLayer` / `DeleteLayer` | the detached subtree, with its original ids |
| `MoveLayer` | source and destination parent + index |
| `SetLayerProperty` | old and new value, and coalesces across a slider drag |
| `SetLayerMask`, `SetSelection` | old and new buffers |
| `ResizeCanvas` | the pixels cropping would discard |
| `SetLayerEffects`, `SetAdjustment` | the whole stack / the adjustment, before and after, coalescing across a drag |
| `Transaction` | a batch that applies and reverses as one, rolling back on failure |
| `SetRigCommand` | the rig before and after (vertex and keyform data, not pixels); parameter values and simulation output are deliberately excluded, so undo never moves the sliders |

Interactive tools paint live and only build their command when the gesture
ends; `History::push_applied` records such a command without re-running it.

## 7. Threading

Today: the UI thread runs input, layout and command execution; `rayon` fans the
compositing and filter kernels across rows. Painting a stroke never blocks on
I/O.

Planned (Phase 10): a dedicated render thread, and a worker pool for
thumbnails, export encoding, asset loading and autosave.

## 8. Files

`.aether` is a ZIP archive holding `project.json` plus one PNG per raster layer
and per mask. The manifest carries a schema version and passes through a
migration step on load. See [docs/FILE_FORMAT.md](docs/FILE_FORMAT.md).

Saves are written to a temporary sibling and renamed into place, so a crash
mid-save cannot destroy the previous version.

## 8b. Interchange

Layered PSD is read (raw and RLE channels, folders, masks, Unicode names) and
written, because that is how artwork arrives for rigging. Animation exports
to GIF, PNG sequences and sprite sheets with a JSON atlas, and rigged
characters export as runtime models (section 5d). Live2D motions and
expressions are read and written, translating curves exactly where both
formats have the shape and fitting Bézier runs where only Aether does.

## 9. Extensibility seams

These exist now, so later phases plug in rather than rewrite:

- `LayerContent::Custom` + `CustomContentRenderer` — plugin layer types
- `Tool` trait + `ToolBox::register` — plugin tools
- `Adjustment` — serialisable, non-destructive colour operations
- `Language::tr` — every UI string looked up by key
- `ShortcutMap` — actions are data, not `match` arms in widget code

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| CPU compositing will not scale to 8K canvases with dozens of layers | Tile-granular dirty tracking is already in place; Phase 8 moves the composite to a GPU render graph behind the same interface, with the CPU path kept as the reference |
| Desktop tablet APIs (Wintab, Windows Ink) are not exposed by `winit` | `InputSample` carries pressure/tilt/velocity throughout; the UI uses the device force `winit` *does* forward for touch-style digitisers, and offers a speed-derived fallback. Adding a platform backend later changes one function, not the pipeline |
| Undo memory growth on large canvases | Commands store rectangles and tiles, not layers; the history has a bounded depth |
| A single enum of layer types would block plugins | `LayerContent::Custom` preserves unknown content across save/load |
| File-format lock-in | Open container, documented schema, versioned with a migration hook |
| Feature breadth outrunning quality | Phases ship complete and tested; nothing lands as a disabled button |
