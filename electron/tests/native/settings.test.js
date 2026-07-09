import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, settingsState, openSettingsWindow } from './fixtures/electron.js';

// Bug 1: "everything in a gray frame, and on first open only half is visible."
// Two native defects:
//   (a) the window has no backgroundColor and the renderer's <html>/<body> are
//       transparent → the default white/gray window shows through ("серая рамка");
//   (b) the window uses show:true (default) so it appears BEFORE the renderer
//       paints → a gray, half-rendered first frame ("видна только половина").
// The fix: dark window backgroundColor + dark html/body, and show the window
// only on 'ready-to-show'. The frame/title bar itself stays (spec §9).
describe('settings window — no gray flash, content fully painted', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function openSettings() {
    ({ app } = await launchApp());
    return openSettingsWindow(app);
  }

  it('window has an opaque dark backgroundColor (no white/gray flash)', async () => {
    await openSettings();
    const state = await settingsState(app);
    expect(state).not.toBeNull();
    // Default (unset) is white #FFFFFF. We want the zinc-950 surface.
    expect(state.backgroundColor.toUpperCase()).not.toBe('#FFFFFF');
    expect(state.backgroundColor.toUpperCase()).not.toBe('#FFF');
  });

  it('the page background is opaque (does not reveal the window chrome)', async () => {
    const settings = await openSettings();
    const bg = await settings.evaluate(() => {
      const body = getComputedStyle(document.body).backgroundColor;
      const html = getComputedStyle(document.documentElement).backgroundColor;
      return { body, html };
    });
    // A transparent body lets the gray native window show through. At least one
    // of html/body must paint an opaque dark surface.
    const transparent = (c) => c === 'rgba(0, 0, 0, 0)' || c === 'transparent';
    expect(transparent(bg.body) && transparent(bg.html)).toBe(false);
  });

  it('the form content fits within the client area (nothing clipped)', async () => {
    const settings = await openSettings();
    const fit = await settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
    }));
    expect(fit.scroll).toBeLessThanOrEqual(fit.inner);
  });

  it('all controls are present and visible', async () => {
    const settings = await openSettings();
    for (const id of [
      'settings-hotkey',
      'settings-hotkey-record',
      'settings-interval',
      'settings-autostart',
      'settings-save',
      'settings-cancel',
    ]) {
      expect(await settings.locator(`[data-testid="${id}"]`).isVisible()).toBe(true);
    }
  });
});
