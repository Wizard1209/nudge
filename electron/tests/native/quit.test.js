import { describe, it, expect, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';

describe('quit — must not crash on a destroyed tray', () => {
  let app;
  afterEach(async () => {
    await app?.close().catch(() => {});
    app = undefined;
  });

  it('ticking the tray refresh after the tray is destroyed does not throw', async () => {
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForLoadState('domcontentloaded');

    await expect(
      app.evaluate(() => globalThis.__nudgeTest.tickTrayAfterDestroy()),
    ).resolves.toBeUndefined();
  });

  it('quitting the app exits cleanly without an error dialog', async () => {
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForLoadState('domcontentloaded');

    const stderr = [];
    app.process().stderr?.on('data', (d) => stderr.push(d.toString()));

    await app.evaluate(({ app }) => app.quit());
    await new Promise((r) => setTimeout(r, 1200));

    const text = stderr.join('');
    expect(text).not.toMatch(/Object has been destroyed/);
    expect(text).not.toMatch(/Uncaught|UnhandledPromiseRejection/);
  });
});
