import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, settingsState, openSettingsWindow } from './fixtures/electron.js';

// Bug 1: "everything in a gray frame, and on first open only half is visible."
// Two native defects:
//   (a) the window has no backgroundColor and the renderer's <html>/<body> are
//       transparent → the default white/gray window shows through ("gray frame");
//   (b) the window uses show:true (default) so it appears BEFORE the renderer
//       paints → a gray, half-rendered first frame ("only half is visible").
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

  // §9: the window is sized to the form — nothing clipped, no empty band
  // below the buttons: the button row ends one 20px margin above the bottom.
  it('the window is sized to the form (nothing clipped, nothing to spare)', async () => {
    const settings = await openSettings();
    const fit = await settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      formBottom: document.querySelector('[data-testid="settings-save"]').getBoundingClientRect().bottom,
    }));
    expect(fit.scroll).toBe(fit.inner);
    expect(fit.formBottom).toBeCloseTo(fit.inner - 20, 0);
  });

  // §9: the banner keeps its line above the buttons whether or not it has
  // anything to say, so a refused save moves nothing and scrolls nothing.
  it('a refused save neither moves the buttons nor scrolls the form', async () => {
    const settings = await openSettings();
    const probe = () => settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      saveTop: document.querySelector('[data-testid="settings-save"]').getBoundingClientRect().top,
    }));
    const quiet = await probe();
    // The longest message the banner carries: a bare key, refused with the
    // vocabulary after it.
    await settings.fill('[data-testid="settings-hotkey"]', 'J');
    await settings.click('[data-testid="settings-save"]');
    await settings.waitForSelector('[data-testid="settings-banner"]');
    expect(await probe()).toEqual(quiet);
    expect(quiet.scroll).toBe(quiet.inner);
  });

  it('all controls are present and visible', async () => {
    const settings = await openSettings();
    for (const id of [
      'settings-hotkey',
      'settings-hotkey-record',
      'settings-interval',
      'settings-autostart',
      'settings-sprite-enabled',
      'settings-sprite-path',
      'settings-sprite-browse',
      'settings-sprite-x',
      'settings-sprite-y',
      'settings-card-width',
      'settings-save',
      'settings-cancel',
    ]) {
      expect(await settings.locator(`[data-testid="${id}"]`).isVisible()).toBe(true);
    }
  });
});
