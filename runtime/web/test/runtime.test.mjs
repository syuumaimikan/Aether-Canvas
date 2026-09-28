// The WebAssembly runtime, driven from JavaScript, must pose the model
// exactly like the native player. Run after `runtime/web/build.sh --demo`:
//
//   node --test runtime/web/test/

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { AetherRuntime } from '../aether-player.js';

const web = new URL('../', import.meta.url);
const runtime = await AetherRuntime.instantiate(await readFile(new URL('aether_player.wasm', web)));
const json = await readFile(new URL('model/model.json', web), 'utf8');
const reference = JSON.parse(await readFile(new URL('test/reference/poses.json', web), 'utf8'));

test('a model loads with its parameters, motions and parts', () => {
  const model = runtime.createModel(json);
  assert.equal(model.width, 512);
  assert.equal(model.height, 640);
  assert.deepEqual(model.textureFiles, ['texture_0.png']);
  assert.ok(model.parameters.length >= 20);
  const angle = model.parameters.find((p) => p.name === 'AngleX');
  assert.deepEqual([angle.min, angle.max, angle.default], [-30, 30, 0]);
  assert.ok(model.motions.length >= 1 && model.motions[0].duration > 0);
  assert.equal(model.parts.length, 17);
  for (const part of model.parts) {
    assert.equal(part.uvs.length, part.vertexCount * 2);
    assert.ok(part.indices.every((i) => i < part.vertexCount));
    assert.ok(part.uvs.every((v) => v >= 0 && v <= 1), `${part.name} uvs inside the atlas`);
  }
  model.dispose();
});

test('every reference pose matches the native player', () => {
  const model = runtime.createModel(json);
  for (const pose of reference) {
    model.reset();
    for (const [name, value] of Object.entries(pose.values)) model.setParameter(name, value);
    model.update();
    pose.positions.forEach((expected, part) => {
      const actual = model.positions(part);
      assert.equal(actual.length, expected.length);
      let worst = 0;
      for (let i = 0; i < actual.length; i++) worst = Math.max(worst, Math.abs(actual[i] - expected[i]));
      assert.ok(worst < 1e-3, `${pose.name}, ${model.parts[part].name}: off by ${worst}px`);
    });
  }
  model.dispose();
});

test('motions play, blend and settle the same way every time', () => {
  const run = () => {
    const model = runtime.createModel(json);
    assert.ok(model.playMotion(0));
    assert.ok(model.playing);
    for (let i = 0; i < 120; i++) model.tick(1 / 60);
    const values = model.parameters.map((p) => model.parameter(p.index));
    const hair = model.positions(model.parts.findIndex((p) => p.name === 'Front hair')).slice();
    model.dispose();
    return { values, hair };
  };
  const a = run();
  const b = run();
  assert.deepEqual(a, b, 'deterministic');
  assert.ok(Math.abs(a.values[0]) > 1, 'the motion moved AngleX');
});

test('draw lists, hit testing and expressions work through the module', () => {
  const model = runtime.createModel(json);
  const list = model.drawList();
  assert.equal(list.length, 17);
  assert.ok(list.every((d) => d.opacity >= 0 && d.opacity <= 1));
  const clipped = list.filter((d) => d.mask >= 0);
  assert.ok(clipped.length >= 2, 'irises are clipped to the eye whites');
  for (const d of clipped) assert.equal(model.parts[d.mask].name.startsWith('Eye white'), true);
  // The middle of an iris hits the iris, which is drawn over the face.
  const iris = model.parts.findIndex((p) => p.name === 'Iris L');
  const xy = model.positions(iris);
  let [cx, cy] = [0, 0];
  for (let i = 0; i < xy.length; i += 2) [cx, cy] = [cx + xy[i], cy + xy[i + 1]];
  assert.equal(model.hitTest((2 * cx) / xy.length, (2 * cy) / xy.length)?.name, 'Iris L');
  assert.equal(model.hitTest(256, 330)?.name, 'Face');
  assert.equal(model.hitTest(-10, -10), null);
  // Unknown names are ignored rather than fatal.
  model.setParameter('NoSuchParameter', 1);
  assert.ok(Number.isNaN(model.parameter('NoSuchParameter')));
  assert.equal(model.playMotion('NoSuchMotion'), false);
  model.setExpression(null);
  model.tick(1 / 60);
  model.dispose();
});

test('look-at and lip sync inputs drive their parameters', () => {
  const model = runtime.createModel(json);
  model.lookAt(1, 0);
  model.setAudio(1, 0);
  for (let i = 0; i < 60; i++) model.tick(1 / 60);
  assert.ok(model.parameter('AngleX') > 5, `AngleX ${model.parameter('AngleX')}`);
  assert.ok(model.parameter('MouthOpenY') > 0.3, `MouthOpenY ${model.parameter('MouthOpenY')}`);
  model.setStage('behaviours', false);
  model.reset();
  model.tick(1 / 60);
  assert.equal(model.parameter('MouthOpenY'), 0);
  model.dispose();
});

test('broken models are rejected with a reason', () => {
  assert.throws(() => runtime.createModel('{'), /not a valid model/);
  const bad = JSON.parse(json);
  bad.parts[0].triangles.push(0, 1, 99999);
  assert.throws(() => runtime.createModel(JSON.stringify(bad)), /triangle past its vertices/);
  // The runtime survives and keeps working.
  runtime.createModel(json).dispose();
});
