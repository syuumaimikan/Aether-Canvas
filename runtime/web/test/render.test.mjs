// WebGL must draw what the software player draws. Runs the parity page in
// headless Chromium through Playwright, and is skipped when Playwright is
// not installed. Run after `runtime/web/build.sh --demo`:
//
//   node --test runtime/web/test/*.test.mjs
//
// Set AETHER_SCREENSHOT=path.png to keep a screenshot of the final frame.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { serve } from './serve.mjs';

async function loadPlaywright() {
  try {
    return await import('playwright');
  } catch {
    // Fall back to a global install.
    try {
      const root = execSync('npm root -g', { encoding: 'utf8' }).trim();
      return createRequire(`${root}/`)('playwright');
    } catch {
      return null;
    }
  }
}

const playwright = await loadPlaywright();

test('WebGL renders every pose like the software player', { skip: !playwright && 'Playwright is not installed' }, async () => {
  const server = await serve(fileURLToPath(new URL('../', import.meta.url)));
  const browser = await playwright.chromium.launch({
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const page = await browser.newPage({ deviceScaleFactor: 1 });
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    await page.goto(`http://127.0.0.1:${server.address().port}/test/render.html`);
    const webgl2 = await page.evaluate(() => window.runParity({ webgl: 2 }));
    const webgl1 = await page.evaluate(() => window.runParity({ webgl: 1 }));
    assert.deepEqual(errors, []);
    assert.equal(webgl2.webgl2, true);
    assert.equal(webgl1.webgl2, false);
    const results = [...webgl2.results, ...webgl1.results];
    assert.equal(results.length, 8);
    for (const r of results) {
      // GPUs filter with limited sub-texel precision and premultiply textures
      // in 8 bits, so small differences at soft edges are expected; a
      // misplaced vertex or a wrong blend would show up as large, widespread
      // ones.
      assert.ok(r.mean < 0.1, `${r.name}: mean difference ${r.mean}`);
      assert.ok(r.over8 / r.pixels < 0.0005, `${r.name}: ${r.over8} channels differ by more than 8`);
    }
    if (process.env.AETHER_SCREENSHOT) {
      await page.locator('#canvas').screenshot({ path: process.env.AETHER_SCREENSHOT });
    }
  } finally {
    await browser.close();
    server.close();
  }
});

test('the demo page plays, follows the pointer and reports taps', { skip: !playwright && 'Playwright is not installed' }, async () => {
  const server = await serve(fileURLToPath(new URL('../', import.meta.url)));
  const browser = await playwright.chromium.launch({
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1100, height: 720 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => document.querySelectorAll('#parameters input').length > 0);
    assert.ok(await page.locator('#message').isHidden());

    // Parameter sliders track the drawn values; the idle motion moves them.
    const angle = page
      .locator('#parameters .param')
      .filter({ has: page.getByText('AngleX', { exact: true }) })
      .locator('output');
    const box = await page.locator('#canvas').boundingBox();
    // Far right of the stage: the head turns right (AngleX goes positive).
    await page.mouse.move(box.x + box.width - 5, box.y + box.height / 3);
    await page.waitForFunction(
      (el) => Number(el.textContent) > 10,
      await angle.elementHandle(),
      { timeout: 5000 },
    );

    // Tap the middle of the body.
    await page.mouse.click(box.x + box.width / 2, box.y + box.height * 0.85);
    await page.waitForFunction(() => document.getElementById('status').textContent.startsWith('touched'));
    assert.match(await page.locator('#status').textContent(), /touched Body/);
    assert.deepEqual(errors, []);
    if (process.env.AETHER_DEMO_SCREENSHOT) {
      await page.screenshot({ path: process.env.AETHER_DEMO_SCREENSHOT });
    }
  } finally {
    await browser.close();
    server.close();
  }
});

test('the player records what it shows as a video', { skip: !playwright && 'Playwright is not installed' }, async () => {
  const server = await serve(fileURLToPath(new URL('../', import.meta.url)));
  const browser = await playwright.chromium.launch({
    args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 900, height: 600 }, deviceScaleFactor: 1 });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => window.demo.player);
    const result = await page.evaluate(async () => {
      const { player } = window.demo;
      const recording = player.record({ fps: 30 });
      await new Promise((r) => setTimeout(r, 1500));
      const blob = await recording.stop();
      // Play it back to prove it decodes.
      const video = document.createElement('video');
      video.muted = true;
      video.src = URL.createObjectURL(blob);
      await new Promise((resolve, reject) => {
        video.onloadedmetadata = resolve;
        video.onerror = () => reject(new Error('the recording does not decode'));
      });
      return {
        type: blob.type,
        size: blob.size,
        size2: [video.videoWidth, video.videoHeight],
        canvas: [player.canvas.width, player.canvas.height],
        mimeType: recording.mimeType,
      };
    });
    assert.match(result.type, /^video\//);
    assert.ok(result.size > 2000, `a real video: ${JSON.stringify(result)}`);
    assert.deepEqual(result.size2, result.canvas, 'the video is the canvas');
  } finally {
    await browser.close();
    server.close();
  }
});
