import { describe, it, expect, beforeAll, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';
import { macosInput } from './fixtures/macosInput.js';

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
