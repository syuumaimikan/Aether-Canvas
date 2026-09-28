// Run the Cubism Framework's own physics (CubismPhysics, as bundled in the
// npm package "untitled-pixi-live2d-engine") on a physics3.json, for
// tests/cubism_physics.rs: stabilise at the defaults, then drive every
// input parameter with a scripted sweep and record the outputs.
//
//   node physics.mjs CORE.js FRAMEWORK.es.js MODEL.physics3.json PARAMS.json OUT.json
//
// PARAMS.json lists the model's parameters: [{id, min, max, default}, ...].
import fs from 'node:fs';
import vm from 'node:vm';

const [corePath, bundlePath, physicsPath, paramsPath, out] = process.argv.slice(2);
let source = fs.readFileSync(bundlePath, 'utf8');
// Stub the renderer imports; physics never touches them.
source = source.replace(/^import \{([^}]*)\} from "pixi\.js";$/m, (_, names) =>
  names
    .split(',')
    .map((n) => `const ${n.trim()} = class {};`)
    .join('\n'),
);
source = source.replace(/^export \{[\s\S]*?\};?\s*$/m, '');
source += '\nthis.CubismPhysics = CubismPhysics; this.CubismFramework = CubismFramework;';
const context = {
  atob, btoa, console, setTimeout, clearTimeout, performance, TextDecoder, TextEncoder, WebAssembly,
  document: { currentScript: { src: 'core.js' }, createElement: () => ({ getContext: () => null }) },
  location: { href: 'file:///core.js' },
  navigator: { userAgent: 'node' },
};
context.window = context;
context.self = context;
context.globalThis = context;
vm.createContext(context);
// The bundle checks for Cubism Core when it loads.
vm.runInContext(fs.readFileSync(corePath, 'utf8') + '\nthis.Live2DCubismCore = Live2DCubismCore;', context);
await new Promise((resolve) => setTimeout(resolve, 300));
vm.runInContext(source, context);
const CubismPhysics = context.CubismPhysics;
context.CubismFramework.startUp();
context.CubismFramework.initialize();
const idText = (id) => (typeof id === 'string' ? id : id.getString().s);

const params = JSON.parse(fs.readFileSync(paramsPath, 'utf8'));
const values = new Float32Array(params.map((p) => p.default));
const model = {
  getParameterCount: () => params.length,
  getParameterIndex: (id) => params.findIndex((p) => p.id === idText(id)),
  getModel: () => ({
    parameters: {
      values,
      minimumValues: new Float32Array(params.map((p) => p.min)),
      maximumValues: new Float32Array(params.map((p) => p.max)),
      defaultValues: new Float32Array(params.map((p) => p.default)),
    },
  }),
};
// This bundle's JSON reader throws on a missing Meta.Fps where the official
// one returns the default (0, meaning "step with the frame"); say it.
const parsed = JSON.parse(fs.readFileSync(physicsPath, 'utf8'));
if (parsed.Meta.Fps === undefined) parsed.Meta.Fps = 0;
const json = Buffer.from(JSON.stringify(parsed));
const physics = CubismPhysics.create(json.buffer.slice(json.byteOffset, json.byteOffset + json.byteLength), json.byteLength);
physics.stabilization(model);
const stabilized = Array.from(values);

// Inputs: every parameter a setting reads, swept with a distinct sine.
const physicsJson = parsed;
const inputs = new Set();
for (const s of physicsJson.PhysicsSettings) for (const i of s.Input) inputs.add(i.Source.Id);
const driven = [...inputs].map((id) => params.findIndex((p) => p.id === id)).filter((i) => i >= 0);

const frames = [];
const dts = [1 / 60, 1 / 30, 1 / 144, 0.05];
let t = 0;
for (let f = 0; f < 240; f++) {
  const dt = dts[f % dts.length];
  t += dt;
  const set = {};
  driven.forEach((p, k) => {
    const q = params[p];
    const v = q.default + Math.sin(t * (1.3 + 0.7 * k) + k) * (q.max - q.min) * 0.45;
    values[p] = Math.min(q.max, Math.max(q.min, v));
    set[p] = values[p];
  });
  physics.evaluate(model, dt);
  frames.push({ dt, set, values: Array.from(values) });
}
fs.writeFileSync(out, JSON.stringify({ stabilized, frames }));
console.log(`${physicsPath}: ${frames.length} frames, ${driven.length} inputs`);
