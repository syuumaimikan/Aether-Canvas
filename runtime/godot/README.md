# Aether Player for Godot 4

A GDExtension that adds one node, **`AetherModel2D`**, to Godot 4.2 and
later. Point it at a model exported from Aether Canvas and it plays it,
using the same rig code as the editor, compiled in Rust. Keyforms, deformers,
bones and IK, physics, drivers, motions, expressions, blinking, breathing,
look-at, lip sync and face tracking all move as they did while you rigged.

![The demo scene in Godot](../../docs/images/godot-demo.png)

## Try it

```sh
runtime/godot/build.sh          # builds the extension and copies in the demo model
godot --path runtime/godot      # the character follows the mouse; click the face
```

`build.sh --release` makes an optimised build. The extension library ends up
in `addons/aether/bin/` next to `aether.gdextension`.

## Use it in your project

1. Copy `addons/aether/` (with its `bin/`) into your project.
2. Export your character from the editor: **File ▸ Export runtime model…**
   (or `aether-canvas --export-model DIR file.aether`). Copy the folder,
   which holds `model.json` and `texture_*.png`, into the project.
3. Add an **AetherModel2D** node and set **Model Path** to the `model.json`.

The node draws the model with its top-left corner at the node's origin, in
model pixels, so scale and move it like any `Node2D`. Then script it:

```gdscript
@onready var model: AetherModel2D = $Model

func _ready() -> void:
    model.play_motion("Idle", false)
    model.motion_event.connect(func(name, motion): print(motion, ": ", name))

func _process(_delta: float) -> void:
    # Look at the mouse: -1..1 on each axis, y up.
    var size := model.get_canvas_size()
    var p := model.get_local_mouse_position()
    model.look_toward(Vector2(p.x / size.x * 2 - 1, 1 - p.y / size.y * 2))

func _unhandled_input(event: InputEvent) -> void:
    if event is InputEventMouseButton and event.pressed:
        if model.hit_test(model.get_local_mouse_position()) == "Face":
            model.play_motion("Greeting", false)
            model.set_expression("Smile")
```

### API

| Member | |
| --- | --- |
| `model_path`, `autoplay`, `speed`, `playing` | Exported properties: the model, a motion to start on load, playback speed, and whether time advances each frame |
| `load(path) -> bool` | Load another model (false, with the reason printed, when it cannot) |
| `is_loaded()`, `get_canvas_size()` | |
| `get_parameter_names()`, `set_parameter(name, value)`, `get_parameter(name)` | Base values; motions, physics and behaviours layer on top. Unknown names read as `NAN` |
| `get_motion_names()`, `play_motion(name, additive) -> bool`, `stop_motions()`, `is_motion_playing()` | |
| `set_expression(name)` | Fade to an expression; `""` fades out of all of them |
| `look_toward(Vector2)`, `look_ahead()` | Head and eyes follow a point (-1..1, y up) |
| `set_audio(level, brightness)` | Lip sync from a loudness (0..1) and brightness (-1..1), for example from an `AudioEffectSpectrumAnalyzer` |
| `track_face(yaw, pitch, roll, shapes)`, `calibrate_tracking()`, `stop_tracking()` | Drive the face from a tracker: head angles in degrees and a `Dictionary` of ARKit/MediaPipe blend shapes (`{"jawOpen": 0.4, "eyeBlinkLeft": 1.0}`). Mirrored like a webcam by default |
| `hit_test(Vector2) -> String` | The topmost part under a point in the node's coordinates, or `""` |
| `advance(seconds)`, `refresh()`, `reset_pose()` | Step time by hand, redraw after setting parameters, go back to rest |
| `motion_event(name, motion)` | Signal: a motion passed one of its timeline events |
| `model_loaded()` | Signal |

## How it draws

Each part is a canvas item holding its deformed mesh, drawn with a small
canvas shader that applies the part's opacity and multiply and screen tints
to its premultiplied texture. The blend modes are exact over opaque artwork,
the same as the software player:

* normal is premultiplied source-over;
* multiply and add use Godot's `blend_mul` and `blend_add`;
* screen, which Godot's fixed blend modes cannot express in one pass, is
  drawn in two: multiply by (1 − source), then add the source.

A clipped part draws inside a `CLIP_ONLY` canvas group shaped by its base,
at the base's current opacity. That gives exactly the software player's
clipping: coverage times the base's coverage.

It draws the same with all three renderers: Compatibility, Mobile and
Forward+. The extension is plain godot-rust with no platform-specific code,
so `cargo build` in `rust/` should work on Windows and macOS too; only
Linux is tested here. For a web page without Godot, use `runtime/web`.

## Tests

```sh
GODOT=/path/to/godot runtime/godot/test.sh
```

This runs `test/run_tests.gd` twice:

* **Headless**, for the node's logic: loading, parameters, hit testing,
  motions and their fade-out, face tracking (mirroring included), and bad
  paths.
* **Under Xvfb, with each renderer** (Compatibility on OpenGL, Forward+ and
  Mobile on Vulkan; Mesa's software drivers are enough), for drawing. The test draws the demo character in four poses and
  a fixture scene in three. The fixture is a tinted, fading, bending mesh
  with a part clipped to it, plus multiply, screen, add and translucent
  parts. Both are compared pixel by pixel with the software player's
  renders.

On Godot 4.3 the mean difference is below 0.13 levels (of 255) with every
renderer. On OpenGL the largest difference in the fixture is a single level.
