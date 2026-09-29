# Aether Canvas user guide

*日本語版: [GUIDE.ja.md](GUIDE.ja.md)*

This guide walks through the editor task by task: painting, bringing in a
PSD, rigging it, animating it and sending it to VTube Studio, a web page or
a game. For the ideas behind rigging see [RIGGING.md](RIGGING.md); for the
runtime model format and players see [RUNTIME.md](RUNTIME.md).

Inside the editor, **Help › Tutorials** runs short guided versions of the
same steps. Each step notices when it is done, and *Show me* does it for you.

## Contents

1. [Getting started](#getting-started)
2. [The interface](#the-interface)
3. [Painting](#painting)
4. [Layers, effects and adjustments](#layers-effects-and-adjustments)
5. [Files, PSDs, ZIPs and the sample library](#files-psds-zips-and-the-sample-library)
6. [Rigging a character](#rigging-a-character)
7. [Animation](#animation)
8. [Live2D models and VTube Studio](#live2d-models-and-vtube-studio)
9. [Games, apps and the web](#games-apps-and-the-web)
10. [The command line](#the-command-line)
11. [Troubleshooting](#troubleshooting)

## Getting started

Aether Canvas is built from source with a stable Rust toolchain (1.82 or
newer, from [rustup.rs](https://rustup.rs)):

```sh
git clone https://github.com/syuumaimikan/Aether-Canvas
cd Aether-Canvas
cargo run --release -p aether-desktop                  # start the editor
cargo run --release -p aether-desktop -- art.aether    # open a file
```

The editor needs a GPU driver with Vulkan, Metal, DirectX 12 or OpenGL.
On Linux it also needs `libxkbcommon` and the X11 or Wayland client
libraries.

On first start the status bar points to the tutorials. **Window ›
Language** switches between English and Japanese; **Window › Theme**
between dark, light and high contrast.

## The interface

* **Menus** along the top: File, Edit, Layer, Filter, Image, Rig, View,
  Workspace, Window, Help.
* **Panels** around the canvas: Tools, Tool Options, Layers, Color, Brush,
  History, Properties, Effects, Parameters, Rig, Dynamics, Timeline. Drag a
  panel by its tab to move it, drop it on the edge of another to split the
  area, or on its tab bar to stack them.
* **Workspaces** (Workspace menu) are panel layouts for one kind of work:
  *Illustration*, *Pixel Art* (pixel grid, hard brush, nearest-neighbour
  view), *Compositing*, *Rigging* and *Animation*. Switching workspace never
  touches the document.
* **The status bar** shows the tool, zoom, canvas size, active layer and the
  last message — what an operation did, or what went wrong.

### Moving around the canvas

| Do | How |
| --- | --- |
| Zoom | Mouse wheel, `Ctrl` + `+` / `Ctrl` + `-` |
| Pan | Hold `Space` or the middle button and drag; or the Pan tool (`H`) |
| Fit to the window / actual size | `Ctrl+0` / `Ctrl+1` |
| Rotate the view | `Shift+,` and `Shift+.`; `Shift+/` resets |
| Mirror the view | `Shift+M` |
| Pixel grid | `Ctrl+G` |

### Keyboard shortcuts

| Action | Key | Action | Key |
| --- | --- | --- | --- |
| Brush | `B` | Undo / Redo | `Ctrl+Z` / `Ctrl+Shift+Z` |
| Eraser | `E` | New layer | `Ctrl+Shift+N` |
| Bucket fill | `G` | Duplicate layer | `Ctrl+J` |
| Eyedropper | `I` | Merge down | `Ctrl+M` |
| Rectangle select | `M` | Select all / none | `Ctrl+A` / `Ctrl+D` |
| Move | `V` | Invert selection | `Ctrl+Shift+I` |
| Transform | `Ctrl+T` | Save / Save as | `Ctrl+S` / `Ctrl+Shift+S` |
| Liquify | `Shift+L` | Export PNG | `Ctrl+E` |
| Brush smaller / larger | `[` / `]` | Swap colours | `X` |
| Mesh tool | `U` | Play / pause | `P` |
| Deform tool | `W` | Previous / next frame | `,` / `.` |
| Bone tool | `K` | Key all parameters | `Shift+K` |
| Physics preview | `Shift+P` | Reset pose | `Shift+R` |

Every shortcut can be changed in **Help › Keyboard Shortcuts**; conflicts
are pointed out, and *Restore defaults* puts them back.

## Painting

1. Choose the **Brush** (`B`). *Tool Options* sets size, opacity, flow,
   hardness, spacing and the stabiliser (which smooths shaky lines); the
   **Brush** panel holds presets and tips, including paper textures and
   image stamps. Save your own presets there and share them as JSON files.
2. **Pen pressure** drives size (and whatever else you map it to) where the
   system reports it. *Pressure source* can use stroke speed instead, or be
   switched off.
3. Colours come from the **Color** panel (primary and secondary, hex, the
   palette and recent colours). `X` swaps them; the eyedropper (`I`) picks
   from the canvas.
4. The **eraser** (`E`) uses the brush's shape. The **bucket** (`G`) fills
   by similarity; its tolerance and *sample all layers* are in Tool Options.
5. **Selections** (rectangle, ellipse, lasso, magic wand) limit painting,
   filling, filters and transforms. Replace, add, subtract and intersect
   modes are in Tool Options.
6. **Transform** (`Ctrl+T`) scales, rotates, skews, distorts and warps the
   active layer in one cage. `Enter` applies, `Esc` cancels; the whole
   session is one undo step. **Liquify** (`Shift+L`) pushes, twirls,
   pinches and bloats pixels.

Every change is an undoable step. The **History** panel lists them; click
an entry to go back to that point.

## Layers, effects and adjustments

* The **Layers** panel adds, groups, duplicates, deletes and reorders
  layers. Each layer has opacity, a blend mode (23 of them), visibility,
  lock, transparency lock and *clip to layer below*.
* **Groups** composite their children together, or pass through to blend
  each child with what is below.
* **Masks** hide parts of a layer without erasing it (*Add mask from
  selection*).
* **Effects** (the *Effects* panel) are non-destructive: blur, motion blur,
  sharpen, glow, drop shadow, outline, colour overlay, grain and colour
  adjustments. They can be reordered and switched off, and are saved as
  settings rather than pixels.
* **Adjustment layers** (Layer › New Adjustment Layer) — levels, curves,
  hue/saturation, exposure, colour balance and more — change everything
  below them without touching it.
* The **Filter** menu applies the same effects destructively, so baking an
  effect gives exactly what you saw.

## Files, PSDs, ZIPs and the sample library

| File | Open | Save / export |
| --- | --- | --- |
| `.aether` project | File › Open | File › Save — a ZIP of JSON and PNG ([FILE_FORMAT.md](FILE_FORMAT.md)) |
| PNG, JPEG, WebP, TIFF, BMP, GIF | File › Open (as a new document) | File › Export PNG (PNG, JPEG, WebP) |
| Layered PSD | File › Open | File › Export PSD |
| Live2D model (`.model3.json`) | File › Open Live2D model… | File › Export Live2D model… |
| ZIP holding any of these | File › Open, or the sample library | — |

Files can also be dropped on the window.

**PSDs** keep their folders, blend modes, opacity, clipping, masks and
Japanese layer names in both directions.

### ZIP archives

A ZIP opens without unpacking. If it holds one model, PSD, project or
image, that opens straight away; if it holds several, a window lists them
with their sizes and you pick one. ZIPs inside ZIPs are looked into, and
file names written by Japanese Windows (Shift_JIS) are read correctly. A
model's textures are not listed separately. Cubism Editor projects
(`.cmo3`, `.can3`) cannot be opened; their exported runtime models
(`.model3.json`) can.

### The sample library

**File › Sample library…** lists everything that can be opened in a folder
of samples — loose files and the contents of every ZIP in it — with a
filter and one-click opening:

1. Put your downloads in a folder named `assets_sample` in the folder you
   start the editor from (the repository folder, with `cargo run`), or next
   to the application. The `AETHER_SAMPLES` environment variable points
   elsewhere; *Choose folder…* picks any folder while the editor runs.
2. Each row shows the kind (Live2D model, PSD, image), the size, the name
   and where it is (`pack.zip › runtime`). Filter by name or kind.
3. **Open** opens it. Tick *Auto rig PSDs when opened* to rig PSDs as soon
   as they load. Large PSDs take a few seconds; the editor stays usable
   meanwhile, and the status bar says what is opening.

`assets_sample/` is ignored by git: sample characters come with their own
licences, which usually do not allow redistributing them.

## Rigging a character

A rig turns layers into a character that moves: **parameters** (`AngleX`,
`EyeLOpen`, `MouthOpenY` …) drive **keyforms** — shapes stored at key
values — on meshes, **deformers** (warps and rotations) and **bones**.
[RIGGING.md](RIGGING.md) explains each piece.

### The quick way: auto rig

1. Open a PSD whose layers are the parts: face, eye whites, irises, lashes,
   brows, mouth, hair (front, side, back), body. Folders help.
2. Choose **Rig › Auto rig** (or *✨ Auto rig* in the Rigging workspace). It
   meshes every layer and builds, as one undo step:
   * a 3D-looking **head turn** (`AngleX`, `AngleY`), head tilt (`AngleZ`),
     body turn and tilt, and **breathing**;
   * **blinking** eyes and **smiling** eyes, **irises** that look around
     (`EyeBallX/Y`), **brows**, a **mouth** that opens and smiles, a
     **blush**;
   * **hair and accessories** that sway with physics;
   * auto-blink, look-at, a body that follows the head, and an **Idle**
     motion.
3. Drag the sliders in the **Parameters** panel to try it, and turn on
   **Physics preview** (`Shift+P`) to see blinking, breathing and swaying
   live.

The status bar says how many parts were recognised and names any left
static. **Renaming a layer and running Auto rig again** is usually the
quickest fix: it rebuilds the rig from scratch each time.

#### Naming layers

Names are read in English or Japanese, case-insensitively, with left and
right from `L`/`R`, `left`/`right` or `左`/`右` (the character's left, on the
viewer's right). The full list is in
[RIGGING.md › Auto rig](RIGGING.md#auto-rig); the common ones:

| Part | Names |
| --- | --- |
| Face | face, 顔, 輪郭, 肌 |
| Eye white / iris / lashes | 白目 · 瞳, 目玉, 黒目 · まつげ, 二重 |
| Brows, mouth, blush | 眉, まゆ · 口, 口 開き, 舌 · 頬, 照れ |
| Hair | 前髪 · 横髪, もみあげ · 後ろ髪, ポニー, テール · アホ毛 |
| Body and clothes | 体, 服, 上着, シャツ, スカート, 脚, 腕, 手 |
| Left out | 原画, 下書き, ラフ, 背景, bg |

PSDs split into materials, like the Live2D samples, work as they are:

* **Generic layer names** (`線`, `塗り`, `影`, `Layer 3`) take the role of
  their folder: `前髪/線` is bangs.
* **A/B alternatives** (`腕A_L`, `腕B_L`: arms down or crossed) get a
  **Variant** parameter: A at 0, B at 1.
* A detail drawn across **both eyes** on one layer follows each eye;
  clipping masks inside an eye close with it.
* **Masks move with the parts** they trim.

### Refining by hand

* **Pick an object** in the **Rig** panel (the tree of deformers, bones and
  meshes) or by clicking the canvas.
* **Mesh tool** (`U`): add, move and remove vertices; *Rig › Auto mesh*
  regenerates the active layer's mesh and *Mesh all layers* every one. The
  rest pose is shown while the tool is active.
* **Keyforms**: in the Parameters panel, put the object's parameter on a key
  (the slider snaps to the ◊ marks), then shape it with the **Deform tool**
  (`W`). Right-click a parameter to add keys at min/default/max, mirror a
  keyform to the other side, add a blend shape or a driver.
* **Deformers**: *Rig › + Warp deformer* and *+ Rotation deformer* wrap
  the selected objects. The Rig panel's generators fill a warp with a
  *3D head turn*, a sway or a squash.
* **Bones** (`K`): draw a chain, then skin meshes to it; IK constraints
  pull chains toward targets.
* **Physics** (the Dynamics panel): pendulum chains driven by head and body
  angles, with gravity, wind, stiffness, limits and colliders.
  *Standard physics & behaviours* adds hair chains, blinking and breathing
  for the standard parameters.
* **Drivers** compute one parameter from others — `BodyAngleX = self +
  AngleX * 0.3` makes the body follow the head.

## Animation

1. Switch to the **Animation** workspace: the **Timeline** sits under the
   canvas.
2. **+ Motion** creates a motion (three seconds by default; change its
   length, frame rate and looping in the timeline).
3. Turn on **Animate** and **Auto key**. Click the ruler to move the
   playhead and drag parameters: each change writes a key. `Shift+K` keys
   every parameter. Drag a key to retime it, select it to choose its easing
   (step, linear, Bézier, ease, back, elastic, bounce, spring).
4. `P` plays; `,` and `.` step frames. With physics preview on, hair and
   clothes follow the motion.
5. **Lip sync**: *Load WAV…* then *Bake lip sync* writes mouth keys from a
   voice recording.
6. **Expressions** are saved parameter sets (Smile, Angry …) blended on top
   of motions. In the **Dynamics** panel, *Capture* saves every parameter
   that is away from its default as an expression; click one to preview it.
7. **Export**: Rig › *Export GIF…*, *Export APNG…* (full colour and soft
   transparency), a PNG sequence or a sprite sheet with a JSON atlas.
   Physics can warm up first, so chains start settled.

Motions and expressions import and export as Live2D `.motion3.json` and
`.exp3.json`, to use Aether's timeline with Live2D models.

## Live2D models and VTube Studio

**Open**: File › *Open Live2D model…* opens a `.model3.json` (or its
folder, or a ZIP holding it). The model is drawn exactly as Cubism draws
it; its parameters, parts, motions, expressions, physics, blinking and lip
sync become ordinary Aether rig data, ready to animate and export again.

**Export**: File › *Export Live2D model…* writes a folder with
`NAME.model3.json`, `NAME.moc3`, textures, physics, display names, motions
and expressions. An opened Live2D model is written back unchanged except
for what you edited; a character rigged in Aether becomes a new Cubism
model, checked vertex by vertex against Cubism's own evaluation, and a
window lists anything approximated.

**In VTube Studio**: copy the exported folder into VTube Studio's
`Live2DModels` folder (in its installation, under `VTube
Studio_Data/StreamingAssets/Live2DModels`), start VTube Studio and pick the
model. Blinking and lip sync are declared in the model, so VTube Studio
drives them; standard parameter names (`ParamAngleX`, `ParamEyeLOpen` …)
connect to face tracking without setup.

## Games, apps and the web

File › *Export runtime model…* writes `model.json` and texture atlases:
open JSON and PNG that the Aether runtime plays exactly as the editor does,
every rig feature included. See [RUNTIME.md](RUNTIME.md) for:

* the **web player** (WebAssembly + WebGL) with pointer following, taps,
  microphone lip sync and **webcam face tracking**;
* the **Godot 4** node `AetherModel2D` and the **Unity** component
  `AetherModel`;
* the **C API** for native engines and apps, and the Rust crates.

## The command line

```text
aether-canvas [OPTIONS] [FILE]
```

`FILE` is a `.aether` project, a PSD, a `.model3.json`, an image, a folder
holding a Live2D model, or a ZIP — `pack.zip#path/inside` names one thing in
it.

| Option | What it does |
| --- | --- |
| `--list [FOLDER\|ZIP]` | List what a folder of samples or an archive holds (default: `assets_sample`), one openable path per line |
| `--export-model DIR` | Write FILE as a runtime model, without opening a window |
| `--export-live2d DIR` | Write FILE as a Live2D Cubism model |
| `--auto-rig` | With an export: rig FILE from its layer names first |
| `-h`, `--help` / `-V`, `--version` | Help and version |

```sh
aether-canvas --auto-rig --export-live2d vtuber character.psd
aether-canvas --list assets_sample
aether-canvas --auto-rig --export-model web "assets_sample/pack.zip#chara/parts.psd"
```

## Troubleshooting

* **The editor does not start on Linux** — install `libxkbcommon-x11` (or
  your distribution's equivalent) and a Vulkan or OpenGL driver; the error
  message names what is missing.
* **Some parts stay still after Auto rig** — the status bar names them.
  Rename those layers (see [Naming layers](#naming-layers)) and run Auto rig
  again, or rig them by hand.
* **Something shows through when the head turns** — a layer behind the face
  (back hair, a neck shadow) may be revealed at wide angles. Select the
  *Head* warp and run the Rig panel's *3D head turn* again with a smaller
  yaw, or trim the hidden layer.
* **Both arm poses show at once** — the PSD holds alternatives that are not
  named A/B. Hide one set of layers, or rename them `…A` / `…B` and rig
  again to switch them with Variant.
* **A big PSD is slow** — a 45 MB, 270-layer PSD takes 20–30 seconds and
  about 2 GB of memory to open and rig. Flattening material layers into one
  layer per part (as PSDs made for Live2D import are) makes it much faster.
* **A Live2D export reports differences** — the export window lists each
  approximation (physics, drivers, parts that differ by more than a few
  pixels) and why; parameters and motions are unaffected.
* **Where are my shortcuts / language / theme?** — Help › Keyboard
  Shortcuts, Window › Language, Window › Theme.
