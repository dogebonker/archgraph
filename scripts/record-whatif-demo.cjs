#!/usr/bin/env node
'use strict';

// Dev-only: playwright-core, Chromium, and img2webp (libwebp tools); no runtime dependencies.
// Set PLAYWRIGHT_MODULE, CHROMIUM_EXECUTABLE, and IMG2WEBP for isolated tooling paths.
// node scripts/record-whatif-demo.cjs map.html demo.webp src/file.ts destination/module
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { spawnSync } = require('node:child_process');

async function main() {
  const [html, output, source, target] = process.argv.slice(2);
  assert(html && output && source && target,
    'Usage: record-whatif-demo.cjs map.html output.webp source-file target-module');
  assert.equal(path.extname(output), '.webp', 'Output must have a .webp extension');
  await fs.mkdir(path.dirname(path.resolve(output)), { recursive: true });
  const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright-core');
  const browser = await chromium.launch({
    headless: true,
    ...(process.env.CHROMIUM_EXECUTABLE ? { executablePath: process.env.CHROMIUM_EXECUTABLE } : {}),
  });
  const frames = await fs.mkdtemp(path.join(os.tmpdir(), 'boundrail-whatif-frames-'));
  const errors = [];
  let frame = 0;
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1,
      reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(pathToFileURL(path.resolve(html)).href);
    await page.locator('#whatif-toggle').waitFor();
    await page.evaluate(() => document.fonts.ready);
    const baseline = await page.locator('#stats-text').textContent();
    const capture = async (copies = 1) => {
      const image = await page.screenshot();
      for (let i = 0; i < copies; i++) {
        await fs.writeFile(path.join(frames, `${String(frame++).padStart(4, '0')}.png`), image);
      }
    };
    await capture(8);
    await page.locator('#whatif-toggle').click();
    await capture(8);
    const points = await page.evaluate(({ source, target }) => {
      const file = files.find(file => file.path === source);
      const destination = mods.findIndex(module => module.id === target);
      if (!file || destination < 0 || file.mod === destination) return null;
      const bounds = canvas.getBoundingClientRect();
      const screen = (x, y) => ({ x: bounds.left + view.x + x * view.k,
        y: bounds.top + view.y + y * view.k });
      return { source: screen(file.x, file.y), target: screen(mx[destination], my[destination]) };
    }, { source, target });
    assert(points, 'Source and a different destination module must exist in the map');
    for (const point of Object.values(points)) {
      assert(point.x > 0 && point.x < 1440 && point.y > 0 && point.y < 960,
        'Drag endpoints must be visible; fit the demo graph before recording');
    }
    await page.mouse.move(points.source.x, points.source.y);
    await capture(4);
    await page.mouse.down();
    for (let step = 1; step <= 12; step++) {
      const progress = step / 12;
      await page.mouse.move(points.source.x + (points.target.x - points.source.x) * progress,
        points.source.y + (points.target.y - points.source.y) * progress);
      await capture();
    }
    await page.mouse.up();
    await page.locator('#whatif-result').waitFor({ state: 'visible' });
    const preview = await page.locator('#whatif-result').innerText();
    assert(preview.includes(source), 'Drop preview must identify the dragged file');
    assert(preview.includes(target), 'Drop preview must identify the destination');
    assert.match(preview, /cycle/i, 'Drop preview must show architecture results');
    await capture(30);
    await page.screenshot({ path: path.resolve(output.replace(/\.webp$/, '.png')) });
    await page.locator('#whatif-copy-cli').click();
    assert((await page.locator('#whatif-cli').inputValue()).includes('boundrail simulate'),
      'Copy action must expose the authoritative CLI command');
    await capture(12);
    await page.locator('#whatif-clear').click();
    assert.equal(await page.locator('#stats-text').textContent(), baseline,
      'Clearing must restore the original architecture summary');
    assert.equal(await page.locator('#whatif-result').textContent(), '',
      'Clearing must remove the preview findings');
    assert(await page.evaluate(source => files.some(file => file.path === source), source),
      'Clearing must restore the original file path');
    await capture(8);
    assert.deepEqual(errors, [], 'Rendered map must not throw browser errors');
    const images = (await fs.readdir(frames)).sort().map(image => path.join(frames, image));
    const result = spawnSync(process.env.IMG2WEBP || 'img2webp', ['-loop', '0', '-lossy',
      '-q', '78', '-m', '6', '-d', '125', ...images, '-o', path.resolve(output)],
    { encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr || 'img2webp failed');
    console.log(JSON.stringify({ output: path.resolve(output), frames: frame,
      seconds: frame / 8, bytes: (await fs.stat(output)).size, source, target, preview }, null, 2));
  } finally {
    await browser.close();
    await fs.rm(frames, { recursive: true, force: true });
  }
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
