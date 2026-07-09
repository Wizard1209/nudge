import { describe, it, expect, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';

// Bug 3: quitting raises an error/crash. The tray icon is refreshed by a
// `setInterval(updateTray, 50)` that is never cleared. On quit Electron
// destroys the Tray; if the timer fires in that window, `tray.setImage` on a
// destroyed Tray throws "Object has been destroyed". The fix: guard updateTray
// against a missing/destroyed tray AND clear the interval on will-quit.
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

    // app.evaluate rejects if the evaluated code throws in the main process.
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
