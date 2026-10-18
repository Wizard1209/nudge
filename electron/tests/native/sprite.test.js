import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launchApp, popupState, openSettingsWindow } from './fixtures/electron.js';
import { opaqueRgbaPng } from '../fixtures/png.js';
import { SPRITE_ERROR } from '../../src/shared/sprite.ts';

const FIXTURE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../e2e/fixtures/sprite-green.png');
// The green fixture placed like the real sprite.
const REAL = { enabled: true, path: FIXTURE, x: -41, y: -277, card_width: 679 };

// config.sprite turns the popup into one window spanning card ∪ sprite: one
// window means the sprite can never outlive the card on close.
describe('popup with a sprite', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  function nudgeDir(config) {
    const dir = mkdtempSync(path.join(os.tmpdir(), 'nudge-native-'));
    if (config) writeFileSync(path.join(dir, 'config.json'), JSON.stringify(config));
    return dir;
  }

  async function launch(config) {
    const dir = nudgeDir(config);
    let popup;
    ({ app, popup } = await launchApp({ NUDGE_DIR: dir }));
    await expect.poll(async () => (await popupState(app))?.visible).toBe(true);
    return { popup, dir };
  }

  /** The popup's renderer page; replaced when the window is recreated. */
  function popupPage() {
    return app.windows().find((p) => /index\.html/.test(p.url()) && !p.isClosed());
  }

  function geometry(page) {
    return page.evaluate(() => {
      const r = (sel) => document.querySelector(sel)?.getBoundingClientRect().toJSON() ?? null;
      return { sprite: r('[data-testid="sprite"]'), card: r('[data-testid="nudge"]') };
    });
  }

  async function size() {
    const { contentBounds } = await popupState(app);
    return [contentBounds.width, contentBounds.height];
  }

  async function hide(page) {
    await page.keyboard.press('Escape');
    await expect.poll(async () => (await popupState(app))?.visible).toBe(false);
  }

  async function show() {
    await app.evaluate(() => globalThis.__nudgeTest.show());
    await expect.poll(async () => (await popupState(app))?.visible).toBe(true);
  }

  /** card ∪ sprite for REAL: the sprite's point size depends on the display scale. */
  async function realSize() {
    const scale = await scaleFactor();
    return [Math.max(41 + 679, 720 / scale), Math.max(277 + 130, 430 / scale)];
  }

  it('sizes the window to card ∪ sprite', async () => {
    await launch({ sprite: REAL });
    expect(await size()).toEqual(await realSize());
  });

  /** Scale factor of the display the popup goes on (the cursor's). */
  function scaleFactor() {
    return app.evaluate(({ screen }) =>
      screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).scaleFactor);
  }

  it('places the sprite at the window origin and the card at (41, 277), 679 wide', async () => {
    const { popup } = await launch({ sprite: REAL });
    await popup.waitForSelector('[data-testid="sprite"]');
    const g = await geometry(popup);
    expect([g.sprite.left, g.sprite.top]).toEqual([0, 0]);
    expect([g.card.left, g.card.top, g.card.width]).toEqual([41, 277, 679]);
  });

  it('draws the sprite at native pixels: 1 PNG pixel = 1 device pixel', async () => {
    const { popup } = await launch({ sprite: REAL });
    await popup.waitForSelector('[data-testid="sprite"]');
    const scale = await scaleFactor();
    const g = await geometry(popup);
    // The 720×430 fixture covers 720×430 device pixels, which is fewer points
    // on a HiDPI display; the <img> is laid out at that point size so
    // Chromium maps it 1:1 and never resamples.
    expect([g.sprite.width, g.sprite.height]).toEqual([720 / scale, 430 / scale]);
    expect(await popup.evaluate(() => {
      const img = document.querySelector('[data-testid="sprite"]');
      return [img.naturalWidth, img.naturalHeight];
    })).toEqual([720, 430]);
  });

  it('sizes the window from the sprite\'s point size, so it follows the display scale', async () => {
    // Below a default card nothing but the sprite sets the height.
    await launch({ sprite: { ...REAL, x: 0, y: 130, card_width: 480 } });
    const scale = await scaleFactor();
    expect(await size()).toEqual([Math.max(480, 720 / scale), 130 + 430 / scale]);
  });

  it('keeps the card at its spec screen spot, or as close as the sprite lets it', async () => {
    await launch({ sprite: REAL });
    const { bounds, workArea } = await app.evaluate(({ screen }) =>
      screen.getDisplayNearestPoint(screen.getCursorScreenPoint()));
    // Spec §1: card top at 25% of the monitor's height + 80, centred on the
    // monitor. The sprite rises 277px above the card; where that is above
    // the work area's top edge the window is pinned there instead, and the
    // card then sits 277px below it.
    const specTop = Math.round(bounds.y + bounds.height * 0.25) + 80;
    // WSLg reports a placeholder position until the window manager places
    // the freshly mapped window, so wait for it to settle.
    await expect.poll(async () => {
      const { contentBounds: b } = await popupState(app);
      return [b.x + 41, b.y + 277];
    }).toEqual([
      Math.round(bounds.x + (bounds.width - 679) / 2),
      Math.max(specTop, workArea.y + 277),
    ]);
  });

  it('shows the plain 480×130 popup when the sprite is disabled', async () => {
    const { popup } = await launch({ sprite: { ...REAL, enabled: false } });
    expect(await size()).toEqual([480, 130]);
    expect(await popup.$('[data-testid="sprite"]')).toBeNull();
  });

  it('falls back to the plain popup on a missing file', async () => {
    const { popup } = await launch({ sprite: { ...REAL, path: path.join(os.tmpdir(), 'no-such-sprite.png') } });
    expect(await size()).toEqual([480, 130]);
    expect(await popup.$('[data-testid="sprite"]')).toBeNull();
  });

  it('falls back to the plain popup on a fully opaque PNG', async () => {
    const dir = mkdtempSync(path.join(os.tmpdir(), 'nudge-opaque-'));
    const opaque = path.join(dir, 'opaque.png');
    writeFileSync(opaque, opaqueRgbaPng());
    const { popup } = await launch({ sprite: { ...REAL, path: opaque } });
    expect(await size()).toEqual([480, 130]);
    expect(await popup.$('[data-testid="sprite"]')).toBeNull();
  });

  it('hides the sprite with the card on Esc — they share one window', async () => {
    const { popup } = await launch({ sprite: REAL });
    await hide(popup);
    const windows = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().length);
    expect(windows).toBe(1);
  });

  it('applies a saved sprite while hidden, and removes it again, without a restart', async () => {
    const { popup, dir } = await launch(null);
    await hide(popup);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: REAL }));
    await expect.poll(size, { timeout: 10_000 }).toEqual(await realSize());
    await show();
    await popupPage().waitForSelector('[data-testid="sprite"]');
    expect((await geometry(popupPage())).card.width).toBe(679);

    await hide(popupPage());
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, enabled: false } }));
    await expect.poll(size, { timeout: 10_000 }).toEqual([480, 130]);
    await show();
    expect(await popupPage().$('[data-testid="sprite"]')).toBeNull();
  });

  it('defers a change made while the popup is open until it hides', async () => {
    const { dir } = await launch(null);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: REAL }));
    // Give the watcher time to fire; the open popup must not change under the user.
    await new Promise((r) => setTimeout(r, 1000));
    expect(await size()).toEqual([480, 130]);
    // Hiding applies the sprite by recreating the window, which can close the
    // page between keydown and keyup: the outcome is checked below instead.
    await popupPage().keyboard.press('Escape').catch(() => {});
    await expect.poll(async () => (await popupState(app))?.visible).toBe(false);
    await expect.poll(size, { timeout: 10_000 }).toEqual(await realSize());
  });

  it('moves the sprite when only its placement changes', async () => {
    const { popup, dir } = await launch({ sprite: REAL });
    await hide(popup);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, x: 0, y: 130, card_width: 480 } }));
    // Sprite hangs below a default card: as wide as the wider of the two,
    // card plus sprite tall, the sprite measured in points.
    const scale = await scaleFactor();
    await expect.poll(size, { timeout: 10_000 }).toEqual([Math.max(480, 720 / scale), 130 + 430 / scale]);
    await show();
    await expect.poll(async () => {
      const g = await geometry(popupPage());
      return g.sprite && [g.sprite.left, g.sprite.top, g.card.left, g.card.top, g.card.width];
    }).toEqual([0, 130, 0, 0, 480]);
  });

  it('Settings blocks saving an enabled sprite that is not a cut-out PNG', async () => {
    const { dir } = await launch(null);
    const opaque = path.join(dir, 'opaque.png');
    writeFileSync(opaque, opaqueRgbaPng());
    const settings = await openSettingsWindow(app);
    await settings.check('[data-testid="settings-sprite-enabled"]');
    await settings.fill('[data-testid="settings-sprite-path"]', opaque);
    await settings.click('[data-testid="settings-save"]');
    await expect.poll(() => settings.textContent('[data-testid="settings-sprite-error"]').catch(() => null))
      .toBe(SPRITE_ERROR.opaque);
    expect(() => readFileSync(path.join(dir, 'config.json'))).toThrow();
  });

  it('Settings saves a valid sprite to config.json', async () => {
    const { dir } = await launch(null);
    const settings = await openSettingsWindow(app);
    await settings.check('[data-testid="settings-sprite-enabled"]');
    await settings.fill('[data-testid="settings-sprite-path"]', FIXTURE);
    await settings.fill('[data-testid="settings-sprite-x"]', '-41');
    await settings.fill('[data-testid="settings-sprite-y"]', '-277');
    await settings.fill('[data-testid="settings-card-width"]', '679');
    await settings.click('[data-testid="settings-save"]');
    await expect.poll(() => settings.textContent('[data-testid="settings-banner"]').catch(() => null)).toBe('Saved');
    expect(JSON.parse(readFileSync(path.join(dir, 'config.json'), 'utf-8')).sprite).toEqual(REAL);
  });
});
