# Aether Player for Unity

`com.aethercanvas.player` is a Unity package (2021.3 or later) that plays
characters rigged in Aether Canvas. The rig runs in the native
`aether_player` library, which is the editor's own rig code. Keyforms,
deformers, bones and IK, physics, drivers, motions, expressions, blinking,
breathing, look-at, lip sync and face tracking all move as they did in the
editor.

## Install

1. Build the native library for your platform and put it in the package:

   ```sh
   runtime/unity/build.sh     # → com.aethercanvas.player/Runtime/Plugins/x86_64/
   ```

2. In Unity: **Window ▸ Package Manager ▸ + ▸ Add package from disk…**, and
   pick `runtime/unity/com.aethercanvas.player/package.json`.

## Use

1. Export your character from the editor with **File ▸ Export runtime
   model…**, and copy the folder (`model.json` + `texture_*.png`) into
   `Assets/`. The package's importer imports the pages at full size,
   uncompressed, without mipmaps and with raw colour, because the runtime
   needs them that way.
2. Select `model.json`, then choose **GameObject ▸ Aether Canvas ▸ Model from
   Selected model.json**. You can also add an **Aether Model** component and
   assign the JSON and textures yourself.

The model shows in the Scene view at rest and plays in Play mode. Script it:

```csharp
using AetherCanvas;
using UnityEngine;

public class Character : MonoBehaviour
{
    public AetherModel model;

    void Start()
    {
        model.PlayMotion("Idle");
        model.MotionEventReached += e => Debug.Log($"{e.Motion}: {e.Name}");
    }

    void Update()
    {
        var mouse = Camera.main.ScreenToWorldPoint(Input.mousePosition);
        model.LookAtWorld(mouse);
        if (Input.GetMouseButtonDown(0) && model.HitTest(mouse) == "Face")
        {
            model.PlayMotion("Greeting");
            model.SetExpression("Smile");
        }
    }
}
```

| `AetherModel` | |
| --- | --- |
| `modelJson`, `textures`, `autoplay`, `speed`, `playing` | The model, a motion to start with, and playback |
| `pixelsPerUnit`, `pivot`, `resolution`, `tint`, `sortingLayerName`, `sortingOrder` | Placement and drawing |
| `showInScene` | Off: draw only into `Output` (a `RenderTexture`, premultiplied), for a `RawImage` with the `AetherCanvas/Display` material, or your own material |
| `lipSyncSource`, `lipSyncGain` | Lip sync from an `AudioSource`. It uses the same loudness and brightness measures as the editor and web player |
| `SetParameter`, `GetParameter`, `PlayMotion`, `SetExpression`, `TrackFace` | |
| `HitTest(world)`, `LookAtWorld(world)`, `WorldToModel`, `ModelToWorld` | |
| `LoadFromFile(path)` | Load a model at run time, for example from `StreamingAssets` |
| `Player` | The full player API (`AetherCanvas.Player`), which works in plain .NET too |
| `MotionEventReached` | Motion timeline events |

## How it draws

`ModelRenderer` draws the model into a render texture with one command
buffer, the way every Aether renderer draws:

* Textures are premultiplied once on the GPU.
* The draw list is drawn back to front with the C API's blend states:
  normal, two-pass multiply, screen and add.
* Clipping uses a mask target, sampled at the same pixel.

Because it renders into its own target, it works with the Built-in pipeline,
URP and HDRP alike. The quad it shows the result on uses a plain unlit
transparent shader (Built-in and URP). Under HDRP, show `Output` with your
own material.

Platforms: whatever you build `aether_player` for. That is desktop
(`cargo build --release -p aether-player`), Android (`cargo ndk`) and iOS
(a static library; the binding switches to `__Internal`). Unity's WebGL
target is not supported. For the web, use `runtime/web`.

## Tests

```sh
runtime/unity/test.sh
```

Unity itself cannot run here without a licence, so the tests cover
everything short of that:

* **The C# binding runs against the native library with .NET 8.** It must
  reproduce the native player exactly: every vertex of every part at the
  four reference poses, plus the draw list, clipping, hit testing, motions,
  expressions, face tracking (mirrored and not), errors, disposal and the
  lip-sync measures.
* **Every script compiles against Unity's own assemblies.** That covers the
  runtime and editor scripts, with `UnityEngine.dll` and `UnityEditor.dll`
  from the Unity3D.SDK package, using Unity's C# version.
* **The HLSL in both shaders compiles** with glslang, for each pass and both
  colour spaces.

The blend states and the mask technique are the ones the web player uses,
and the web player is checked in Chromium against the software renderer.
Drawing inside Unity itself has not been tested yet.
