// Dump what Live2D Cubism Core computes for a model, as the reference the
// Rust evaluator is tested against (tests/cubism_oracle.rs).
//
//   node dump.mjs CORE.js MODEL.moc3 OUT_PREFIX [SETS]
//
// Writes OUT_PREFIX.json (static data and, per parameter set, the values
// used and every per-drawable/per-part output except vertex positions) and
// OUT_PREFIX.bin (every drawable's vertex positions for each set, f32 LE).
// The first set is the model's defaults; the rest are random values within
// each parameter's range, from a fixed seed.

import fs from 'node:fs';
import vm from 'node:vm';

const [corePath, mocPath, out, setsArg] = process.argv.slice(2);
const sets = Number(setsArg ?? 12);

const context = {
  atob, btoa, console, setTimeout, clearTimeout, performance, TextDecoder, TextEncoder, WebAssembly,
  document: { currentScript: { src: 'core.js' }, createElement: () => ({}) },
  location: { href: 'file:///core.js' },
  navigator: { userAgent: 'node' },
};
context.window = context;
context.self = context;
context.globalThis = context;
vm.createContext(context);
vm.runInContext(fs.readFileSync(corePath, 'utf8') + '\nthis.Live2DCubismCore = Live2DCubismCore;', context);
const core = context.Live2DCubismCore;
// The Emscripten module finishes initialising asynchronously.
await new Promise((resolve) => setTimeout(resolve, 300));

const bytes = fs.readFileSync(mocPath);
const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
const moc = core.Moc.fromArrayBuffer(buffer);
if (!moc) throw new Error(`Cubism Core could not load ${mocPath}`);
// The check the Cubism SDK runs on files it did not write itself.
const consistent = moc.hasMocConsistency(buffer.slice(0)) === 1;
const model = core.Model.fromMoc(moc);

// mulberry32: small, seedable, the same everywhere.
let seed = 0x5eed1234;
const random = () => {
  seed |= 0;
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const p = model.parameters;
const d = model.drawables;
const listOf = (a) => Array.from(a);
const info = {
  core: core.Version.csmGetVersion(),
  mocVersion: core.Version.csmGetMocVersion(moc, buffer),
  consistent,
  canvas: model.canvasinfo,
  parameters: { ids: listOf(p.ids), min: listOf(p.minimumValues), max: listOf(p.maximumValues), defaults: listOf(p.defaultValues) },
  parts: { ids: listOf(model.parts.ids), parents: listOf(model.parts.parentIndices) },
  drawables: {
    ids: listOf(d.ids),
    constantFlags: listOf(d.constantFlags),
    textureIndices: listOf(d.textureIndices),
    vertexCounts: listOf(d.vertexCounts),
    indexCounts: listOf(d.indexCounts),
    maskCounts: listOf(d.maskCounts),
    masks: listOf(d.masks).map(listOf),
    parentParts: listOf(d.parentPartIndices),
    // Blend code per drawable, for 5.3 files (the wrapper's array is twice
    // as long as the data).
    blendModes: d.blendModes ? listOf(d.blendModes).slice(0, d.count) : null,
    uvs: listOf(d.vertexUvs).map(listOf),
    indices: listOf(d.indices).map(listOf),
  },
  sets: [],
};

const chunks = [];
for (let s = 0; s < sets; s++) {
  const values = listOf(p.defaultValues);
  if (s > 0) {
    for (let i = 0; i < values.length; i++) values[i] = p.minimumValues[i] + random() * (p.maximumValues[i] - p.minimumValues[i]);
  }
  for (let i = 0; i < values.length; i++) p.values[i] = values[i];
  model.update();
  info.sets.push({
    values,
    opacities: listOf(d.opacities),
    drawOrders: listOf(d.drawOrders),
    renderOrders: listOf(model.renderOrders ?? d.renderOrders),
    offscreenOpacities: model.offscreens ? listOf(model.offscreens.opacities ?? []) : [],
    visible: listOf(d.dynamicFlags).map((f) => f & 1),
    multiply: d.multiplyColors ? listOf(d.multiplyColors) : null,
    screen: d.screenColors ? listOf(d.screenColors) : null,
    partOpacities: listOf(model.parts.opacities),
  });
  for (let i = 0; i < d.count; i++) chunks.push(Buffer.from(new Float32Array(d.vertexPositions[i]).buffer));
}
fs.writeFileSync(`${out}.json`, JSON.stringify(info));
fs.writeFileSync(`${out}.bin`, Buffer.concat(chunks));
console.log(`${mocPath}: ${d.count} drawables, ${p.count} parameters, ${sets} sets`);
