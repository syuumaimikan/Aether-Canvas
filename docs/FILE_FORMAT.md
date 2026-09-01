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
