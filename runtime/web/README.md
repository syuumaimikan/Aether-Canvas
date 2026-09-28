# Aether Player for the web

Plays models exported from Aether Canvas in any browser with WebGL. The rig
— keyforms, deformers, bones and IK, physics, drivers, motions, blinking,
breathing, look-at and lip sync — runs in WebAssembly, compiled from the same
Rust code as the editor, so a model moves exactly as it did while you rigged
it. `aether-player.js` draws the result with WebGL 1 or 2.

## Try it

```sh
runtime/web/build.sh --demo          # builds aether_player.wasm and a demo model
node runtime/web/test/serve.mjs      # then open http://localhost:8080/
```

To play your own character, use **File ▸ Export runtime model…** in the
editor (or `aether-canvas --export-model DIR file.aether`), then press
**Open model folder…** on the demo page, or serve the folder and open
`index.html?model=path/to/model.json`.

A layered PSD straight from a painting app becomes a playable model in one
command, rigged from its layer names:

```sh
aether-canvas --auto-rig --export-model public/model character.psd
```

## Use it

Copy `aether-player.js` and `aether_player.wasm` next to each other, then:

```html
<canvas id="stage" style="width: 480px; height: 600px"></canvas>
<script type="module">
  import { AetherPlayer } from './aether-player.js';

  const player = await AetherPlayer.load(document.getElementById('stage'), 'model/model.json');
  player.playMotion('Idle');
  player.followPointer();              // look at the mouse
  player.enableHitTest();              // taps become 'hit' events
  player.on('hit', ({ part }) => part === 'Face' && player.playMotion('Greeting'));
  player.on('event', ({ name }) => console.log('motion event', name));
  player.start();

  // Drive anything by hand — from face tracking, a game, a slider…
  player.setParameter('AngleX', 15);
  player.setExpression('Smile');
  // …or lip sync from a microphone or an <audio> element.
  player.lipSync(await navigator.mediaDevices.getUserMedia({ audio: true }));

  // Record a clip (WebM; a transparent background stays transparent).
  const recording = player.record();
  // …later
  const video = await recording.stop();   // a Blob to download or upload
</script>
```

## Face tracking

`aether-tracking.js` turns a webcam into a puppet controller: it runs
Google's MediaPipe Face Landmarker in the page (video never leaves the
device) and feeds head angles and the 52 standard blend shapes to the model.

```js
import { startFaceTracking } from './aether-tracking.js';

player.stopMotions();                       // the face drives the head now
const tracking = await startFaceTracking(player);   // asks for the camera
tracking.calibrate();                       // "this is my neutral face"
tracking.stop();
```

The model moves like a mirror image by default (`{ mirror: false }` turns
that off); auto-blink pauses while the eyes are tracked. MediaPipe loads from
a CDN unless you pass `vision`, `wasm` and `model` URLs to self-host it. Any
other tracker (an iPhone's ARKit, a VTuber tracking app) can drive a model the
same way through `model.trackFace({ yaw, pitch, roll, shapes })`; the mapping
onto parameters lives in the player, so it is identical everywhere.

The layers underneath are usable on their own:

| Class | Does | Needs |
| --- | --- | --- |
| `AetherRuntime` | Loads the WebAssembly module | Any JavaScript engine with WebAssembly (browsers, Node) |
| `AetherModel` | Parameters, motions, expressions, look-at, lip sync, physics, the per-frame draw list, hit testing | An `AetherRuntime` |
| `WebGLRenderer` | Draws an `AetherModel` into a WebGL context you own | WebGL 1 or 2 |
| `AetherPlayer` | All of the above on a `<canvas>`, with a frame loop and input helpers | A browser |

So a game engine with its own renderer (PixiJS, Three.js, Babylon.js…) can
take `model.drawList()` and `model.positions(part)` each frame and draw the
triangles itself. Types are in `aether-player.d.ts`.

## Drawing rules

For anyone writing a renderer: textures are straight-alpha PNGs; premultiply
on upload and blend premultiplied. Each draw item names a part, an opacity,
a multiply and a screen tint (`rgb = rgb·multiply; rgb = rgb + screen −
rgb·screen`), a blend mode (normal, multiply, screen, add) and optionally a
mask part: draw it only where the mask part's alpha is. `WebGLRenderer` is a
complete, short example, and `aether_player::cpu` in Rust is the reference
renderer (it matches the editor to within one colour level).

## Tests

```sh
runtime/web/build.sh --demo
node --test runtime/web/test/*.test.mjs
```

* `runtime.test.mjs` checks that the WebAssembly module poses every
  reference pose exactly like the native player, and exercises motions,
  events, hit testing and bad input.
* `render.test.mjs` renders every reference pose with WebGL in headless
  Chromium and compares it with the software renderer, then drives the demo
  page (pointer following, taps) and records a clip, checking that it plays
  back at the canvas's size. It needs Playwright and is skipped without it.
* `tracking.test.mjs` checks head angles from face-mesh landmarks and face
  samples through the module; after `test/fetch-tracking-assets.sh` (MediaPipe
  and a public-domain photo of a face) it also has MediaPipe track that face,
  turned on screen, in headless Chromium, and drives the demo page's webcam
  button through a fake camera.
