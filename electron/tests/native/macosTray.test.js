import { describe, it, expect, beforeAll, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';
import { macosInput } from './fixtures/macosInput.js';

// The daisy as a macOS status item, measured where the user sees it: the
// frame the menu bar gives it (Accessibility, fixtures/macos-input.swift
// `extras`). The bar is 24 pt tall on a plain display, 33 pt under a notch,
// and does not scale an oversized image down: an NSImage whose logical size
// is the bitmap's pixel size overflows the bar both ways. A menu bar icon is
// drawn at ~18 pt, so an item is about as wide as the bar is tall.
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

describe.skipIf(process.platform !== 'darwin')('macOS status item', () => {
  let input;
  let app;
  beforeAll(() => {
    input = macosInput();
  });
  afterEach(async () => {
    await app?.close().catch(() => {});
    app = undefined;
  });

  it('the tray image carries one representation per backing scale, each drawn at 18 pt', async () => {
    ({ app } = await launchApp());
    const reps = await app.evaluate(() => globalThis.__nudgeTest.trayImageReps());
    console.log('[tray image]', JSON.stringify(reps));
    // AppKit picks the representation whose scale matches the display, and
    // resamples when none does. Two exact ones, no resample on either kind.
    expect(reps).toEqual([
      { scale: 1, points: 18, pixels: 18 },
      { scale: 2, points: 18, pixels: 36 },
    ]);
  });

  it('the daisy fits inside the menu bar', async () => {
    ({ app } = await launchApp());
    await sleep(500);
    const items = input.extras(app.process().pid);
    console.log('[status item]', JSON.stringify(items));
    expect(items, 'no status item in the AX tree').toHaveLength(1);
    const [item] = items;
    expect(item.y, 'the item starts above the menu bar').toBeGreaterThanOrEqual(0);
    expect(item.height, 'the item is taller than a menu bar').toBeLessThanOrEqual(24);
    expect(item.width, 'the item is wider than a menu bar icon').toBeLessThanOrEqual(48);
  });
});
