// Face tracking: head pose from landmarks, samples through the WebAssembly
// module, and — with the assets from fetch-tracking-assets.sh and
// Playwright — MediaPipe tracking a real face in headless Chromium.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { AetherRuntime } from '../aether-player.js';
import { headPose } from '../aether-tracking.js';
import { serve } from './serve.mjs';

const web = new URL('../', import.meta.url);

// A few face-mesh points in the person's frame of reference, image axes:
// x to the image's right (the person's left), y down, z away from the camera.
const FACE = {
  1: [0, 0.02, -0.1], // nose tip
  33: [-0.09, -0.04, 0], // outer corner of their right eye
  263: [0.09, -0.04, 0], // outer corner of their left eye
  234: [-0.14, 0.03, 0.06], // their right cheek
  454: [0.14, 0.03, 0.06], // their left cheek
  10: [0, -0.18, 0.03], // forehead
  152: [0, 0.16, 0.03], // chin
};

/** Landmarks of FACE turned by yaw (to their left), pitch (up), roll (to their left shoulder). */
function landmarks({ yaw = 0, pitch = 0, roll = 0 }, aspect = 4 / 3) {
  const r = Math.PI / 180;
  const out = [];
  for (const [i, [x0, y0, z0]] of Object.entries(FACE)) {
    // Turning left brings the nose toward the image's right.
    let [x, y, z] = [x0 * Math.cos(yaw * r) - z0 * Math.sin(yaw * r), y0, x0 * Math.sin(yaw * r) + z0 * Math.cos(yaw * r)];
    // Looking up sends the forehead back and brings the chin forward.
    [y, z] = [y * Math.cos(pitch * r) + z * Math.sin(pitch * r), -y * Math.sin(pitch * r) + z * Math.cos(pitch * r)];
    // Tipping toward their left shoulder moves the top of the head right.
    [x, y] = [x * Math.cos(roll * r) - y * Math.sin(roll * r), x * Math.sin(roll * r) + y * Math.cos(roll * r)];
    out[i] = { x: 0.5 + x / aspect, y: 0.5 + y, z: z / aspect };
  }
  return out;
}

test('head pose comes back from rotated landmarks', () => {
  const aspect = 4 / 3;
  for (const angle of [-25, -10, 10, 25]) {
    for (const axis of ['yaw', 'pitch', 'roll']) {
      const pose = headPose(landmarks({ [axis]: angle }, aspect), aspect);
      assert.ok(Math.abs(pose[axis] - angle) < 0.5, `${axis} ${angle}: got ${JSON.stringify(pose)}`);
      for (const other of ['yaw', 'pitch', 'roll'].filter((a) => a !== axis)) {
        assert.ok(Math.abs(pose[other]) < 1, `${axis} ${angle} leaks into ${other}: ${pose[other]}`);
      }
    }
  }
  // Combined rotations keep their signs.
  const pose = headPose(landmarks({ yaw: 15, pitch: -10, roll: 8 }, aspect), aspect);
  assert.ok(pose.yaw > 10 && pose.pitch < -5 && pose.roll > 5, JSON.stringify(pose));
});

test('face samples drive the model through the module', async () => {
  const runtime = await AetherRuntime.instantiate(await readFile(new URL('aether_player.wasm', web)));
  assert.equal(runtime.blendshapes.length, 52);
  assert.equal(runtime.blendshapes[24], 'jawOpen');
  const model = runtime.createModel(await readFile(new URL('model/model.json', web), 'utf8'));
  // Turned toward their left, mouth open, left eye shut: MediaPipe's shape.
  const sample = {
    yaw: 20,
    shapes: [
      { categoryName: '_neutral', score: 0 },
      { categoryName: 'jawOpen', score: 0.5 },
      { categoryName: 'eyeBlinkLeft', score: 0.9 },
    ],
  };
  for (let i = 0; i < 60; i++) {
    model.trackFace(sample);
    model.tick(1 / 60);
  }
  assert.ok(model.tracking);
  assert.ok(model.parameter('AngleX') < -15, `a mirror image turns left: ${model.parameter('AngleX')}`);
  assert.ok(model.parameter('MouthOpenY') > 0.8);
  assert.ok(model.parameter('EyeLOpen') < 0.1 && model.parameter('EyeROpen') > 0.9);
  // The other two input forms.
  model.trackFace({ shapes: { jawOpen: 0 } });
  model.trackFace({ shapes: new Float32Array(52) });
  model.setTrackingOptions({ mirror: false, smoothing: 0 });
  model.trackFace({ yaw: 20 });
  model.tick(1 / 60);
  assert.ok(model.parameter('AngleX') > 15, `without a mirror it turns right: ${model.parameter('AngleX')}`);
  model.stopTracking();
  model.tick(1 / 60);
  assert.equal(model.tracking, false);
  assert.equal(model.parameter('AngleX'), 0);
  model.dispose();
});

async function trackingAssets() {
  try {
    await access(new URL('test/tracking-assets/face_landmarker.task', web));
    await access(new URL('test/tracking-assets/face.png', web));
    return true;
  } catch {
    return false;
  }
}

async function loadPlaywright() {
  try {
    return await import('playwright');
  } catch {
    try {
      const root = execSync('npm root -g', { encoding: 'utf8' }).trim();
      return createRequire(`${root}/`)('playwright');
    } catch {
      return null;
    }
  }
}

const playwright = await loadPlaywright();
const assets = await trackingAssets();
const skip = !playwright ? 'Playwright is not installed' : !assets ? 'run test/fetch-tracking-assets.sh first' : false;

test('MediaPipe tracks a real face and tilts the model with it', { skip, timeout: 120000 }, async () => {
  const server = await serve(fileURLToPath(web));
  const browser = await playwright.chromium.launch({
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/test/tracking.html`);
    const upright = await page.evaluate(() => window.runTracking(0));
    const tipped = await page.evaluate(() => window.runTracking(20));
    assert.deepEqual(errors, []);
    for (const run of [upright, tipped]) {
      assert.ok(run.faces >= 15, `found the face: ${JSON.stringify(run)}`);
      assert.ok(run.tracking, 'tracking while running');
      assert.equal(run.afterStop.tracking, false);
    }
    // Turning the photo clockwise tips the head toward the person's left
    // shoulder; a mirror image of that tilts counter-clockwise (AngleZ < 0).
    assert.ok(Math.abs(tipped.pose.roll - upright.pose.roll - 20) < 5, `roll ${upright.pose.roll} -> ${tipped.pose.roll}`);
    assert.ok(tipped.angleZ - upright.angleZ < -12, `AngleZ ${upright.angleZ} -> ${tipped.angleZ}`);
    // A 2D turn of the photo is not a head turn.
    assert.ok(Math.abs(tipped.pose.yaw - upright.pose.yaw) < 5, `yaw ${upright.pose.yaw} -> ${tipped.pose.yaw}`);
  } finally {
    await browser.close();
    server.close();
  }
});

test('the demo page tracks a face from the camera', { skip, timeout: 120000 }, async () => {
  const video = fileURLToPath(new URL('test/tracking-assets/face.y4m', web));
  const server = await serve(fileURLToPath(web));
  const browser = await playwright.chromium.launch({
    args: [
      '--use-angle=swiftshader',
      '--enable-unsafe-swiftshader',
      '--ignore-gpu-blocklist',
      '--use-fake-ui-for-media-stream',
      '--use-fake-device-for-media-stream',
      `--use-file-for-fake-video-capture=${video}`,
    ],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 720 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    const assets = 'test/tracking-assets';
    await page.goto(
      `http://127.0.0.1:${server.address().port}/?vision=${assets}/package/vision_bundle.mjs` +
        `&wasm=${assets}/package/wasm&faceModel=${assets}/face_landmarker.task`,
    );
    await page.waitForFunction(() => window.demo.player);
    await page.click('#track');
    await page.waitForFunction(() => window.demo.tracking?.state.faces > 10, null, { timeout: 60000 });
    assert.ok(await page.evaluate(() => window.demo.player.model.tracking));
    assert.ok(await page.locator('#camera').isVisible(), 'the camera preview shows');
    assert.equal(await page.evaluate(() => window.demo.player.model.playing), false, 'tracking stops the idle motion');
    await page.click('#calibrate');
    if (process.env.AETHER_TRACKING_SCREENSHOT) {
      await page.waitForTimeout(500);
      await page.screenshot({ path: process.env.AETHER_TRACKING_SCREENSHOT });
    }
    await page.click('#track');
    assert.equal(await page.evaluate(() => window.demo.tracking), null);
    assert.equal(await page.evaluate(() => window.demo.player.model.tracking), false);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
    server.close();
  }
});
