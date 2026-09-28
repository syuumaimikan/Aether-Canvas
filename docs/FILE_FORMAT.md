# The `.aether` project format

A project file is an ordinary ZIP archive. That is a deliberate choice: artwork
should not be trapped in a format only one program understands. Anyone can
unzip a project, read the manifest and recover every layer as a PNG.

## Layout

```text
project.aether
├── project.json      manifest: schema version, document settings, layer tree
├── layers/<id>.png   one RGBA PNG per raster layer, in document coordinates
├── masks/<id>.png    one 8-bit grayscale PNG per layer mask
├── selection.png     the saved selection, when one is active
├── rig.json          rigging and animation, when the document has any
└── thumbnail.png     a 512px preview for file browsers
```

`<id>` is the numeric layer id, which is stable for the life of the project.

## Manifest

```json
{
  "schema_version": 1,
  "app_version": "0.1.0",
  "document": {
    "name": "Untitled",
    "width": 1920,
    "height": 1080,
    "background": "Transparent",
    "color_model": "Rgba8",
    "active_layer": 1,
    "id_watermark": 7,
    "metadata": { "created": 0, "modified": 0, "author": "", "description": "", "dpi": 72.0 },
    "roots": [1, 4],
    "layers": [
      {
        "id": 1,
        "name": "Layer 1",
        "visible": true,
        "locked": false,
        "alpha_lock": false,
        "opacity": 1.0,
        "blend_mode": "Normal",
        "transform": { "a": 1.0, "b": 0.0, "c": 0.0, "d": 1.0, "tx": 0.0, "ty": 0.0 },
        "clipping": false,
        "mask_enabled": true,
        "color_label": "None",
        "effects": [],
        "mask": null,
        "content": { "Raster": { "data": "layers/1.png" } }
      }
    ],
    "selection": null
  }
}
```

`roots` lists top-level layers bottom-to-top; a group's `content` carries its
own `children` list in the same order.

## Layer content

| Variant | Payload |
| --- | --- |
| `Raster` | `data`: archive path of the PNG |
| `Group` | `children`, `isolate`, `collapsed` |
| `Adjustment` | `adjustment`: the operation and its parameters |
| `Fill` | `color` |
| `Custom` | `kind` (reverse-DNS tag) and an opaque `payload` |

Every layer also carries an `effects` array: the non-destructive effect stack,
stored as parameters (`{"kind": {"Blur": {"sigma": 4.0}}, "enabled": true}`)
rather than as rendered pixels. The field was added after schema version 1
shipped, so files written by an older build simply omit it and load with an
empty stack.

`Custom` is how plugin content survives a round trip through a build that does
not have the plugin: the tag and payload are stored and restored verbatim, and
the layer is simply not drawn.

## Rig

When a document is rigged, the manifest's `document.rig` holds the archive
path `"rig.json"` (older files omit the field). `rig.json` is the serialised
rig:

```json
{
  "parameters": [{ "id": 12, "name": "AngleX", "min": -30.0, "max": 30.0,
                   "default": 0.0, "group": "Face", "cyclic": false }],
  "values": { "12": 18.0 },
  "deformers": [{ "id": 40, "name": "Head", "parent": { "Deformer": 41 },
                  "kind": { "Warp": { "rect": { "min": {"x": 90, "y": 80},
                                                "max": {"x": 420, "y": 520} },
                                      "cols": 8, "rows": 8, "smooth": true,
                                      "keyforms": { "axes": [...], "forms": [...],
                                                    "interpolation": "Smooth" },
                                      "blend_shapes": [] } } }],
  "bones": [],
  "meshes": [{ "layer": 3, "name": "Face", "parent": { "Deformer": 40 },
               "vertices": [x0, y0, x1, y1, ...],
               "triangles": [[0, 1, 2], ...],
               "keyforms": { "axes": [], "forms": [{ "offsets": [0.0, 0.0, ...],
                             "opacity": 1.0, "multiply": [1, 1, 1],
                             "screen": [0, 0, 0], "draw_order": 0.0 }] },
               "blend_shapes": [], "skin": null, "jiggle": null, "glue": [] }],
  "physics": [...], "drivers": [...], "motions": [...],
  "expressions": [...], "behaviours": {...}
}
```

* Point lists (`vertices`, `offsets`) are flat `[x0, y0, x1, y1, …]` arrays
  in document pixels; readers also accept `{"x":…,"y":…}` objects.
* A keyform grid's `forms` has one entry per combination of keys, with axis 0
  varying fastest.
* Mesh vertices are rest positions *and* texture coordinates into the layer's
  PNG.
* `values` is the pose the file was saved in; parameters at their default are
  omitted.

A damaged `rig.json` is reported as an error rather than silently dropped, so
saving over the file cannot destroy rigging work. The full schema is the
serde form of `aether_rig::Rig`.

## Versioning

`schema_version` is checked on load:

- equal to the build's version → loaded directly;
- lower → passed through the migration chain, one step per version;
- higher → rejected with a clear error rather than being partially misread.

## Robustness

- Saves are written to `<name>.aether.tmp` and renamed into place, so an
  interrupted save cannot destroy the previous file.
- A missing layer blob does not fail the load: the layer comes back empty and
  the rest of the document is intact.
- A layer referenced by a broken parent link is reattached at the root instead
  of being dropped.
