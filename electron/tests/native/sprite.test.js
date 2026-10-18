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

  it('sizes the window to card ∪ sprite', async () => {
    await launch({ sprite: REAL });
    expect(await size()).toEqual([720, 447]);
  });

  it('places the sprite at the window origin and the card at (41, 277), 679 wide', async () => {
    const { popup } = await launch({ sprite: REAL });
    await popup.waitForSelector('[data-testid="sprite"]');
    const g = await geometry(popup);
    expect([g.sprite.left, g.sprite.top, g.sprite.width, g.sprite.height]).toEqual([0, 0, 720, 430]);
    expect([g.card.left, g.card.top, g.card.width]).toEqual([41, 277, 679]);
  });

  it('keeps the card at its spec screen spot, or as close as the sprite lets it', async () => {
    await launch({ sprite: REAL });
    const workArea = await app.evaluate(({ screen }) =>
      screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).workArea);
    // The sprite rises 277px above the card. On a work area shorter than
    // 1108px that is above the top edge, and the window is pinned there
    // instead: the card then sits 277px below the top.
    const specTop = Math.round(workArea.y + workArea.height * 0.25);
    // WSLg reports a placeholder position until the window manager places
    // the freshly mapped window, so wait for it to settle.
    await expect.poll(async () => {
      const { contentBounds: b } = await popupState(app);
      return [b.x + 41, b.y + 277];
    }).toEqual([
      Math.round(workArea.x + (workArea.width - 679) / 2),
      Math.max(specTop, workArea.y + 277),
    ]);
  });

  it('shows the plain 480×170 popup when the sprite is disabled', async () => {
    const { popup } = await launch({ sprite: { ...REAL, enabled: false } });
    expect(await size()).toEqual([480, 170]);
    expect(await popup.$('[data-testid="sprite"]')).toBeNull();
  });

  it('falls back to the plain popup on a missing file', async () => {
    const { popup } = await launch({ sprite: { ...REAL, path: path.join(os.tmpdir(), 'no-such-sprite.png') } });
    expect(await size()).toEqual([480, 170]);
    expect(await popup.$('[data-testid="sprite"]')).toBeNull();
  });

  it('falls back to the plain popup on a fully opaque PNG', async () => {
    const dir = mkdtempSync(path.join(os.tmpdir(), 'nudge-opaque-'));
    const opaque = path.join(dir, 'opaque.png');
    writeFileSync(opaque, opaqueRgbaPng());
    const { popup } = await launch({ sprite: { ...REAL, path: opaque } });
    expect(await size()).toEqual([480, 170]);
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
    await expect.poll(size, { timeout: 10_000 }).toEqual([720, 447]);
    await show();
    await popupPage().waitForSelector('[data-testid="sprite"]');
    expect((await geometry(popupPage())).card.width).toBe(679);

    await hide(popupPage());
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, enabled: false } }));
    await expect.poll(size, { timeout: 10_000 }).toEqual([480, 170]);
    await show();
    expect(await popupPage().$('[data-testid="sprite"]')).toBeNull();
  });

  it('defers a change made while the popup is open until it hides', async () => {
    const { dir } = await launch(null);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: REAL }));
    // Give the watcher time to fire; the open popup must not change under the user.
    await new Promise((r) => setTimeout(r, 1000));
    expect(await size()).toEqual([480, 170]);
    // Hiding applies the sprite by recreating the window, which can close the
    // page between keydown and keyup: the outcome is checked below instead.
    await popupPage().keyboard.press('Escape').catch(() => {});
    await expect.poll(async () => (await popupState(app))?.visible).toBe(false);
    await expect.poll(size, { timeout: 10_000 }).toEqual([720, 447]);
  });

  it('moves the sprite when only its placement changes', async () => {
    const { popup, dir } = await launch({ sprite: REAL });
    await hide(popup);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, x: 0, y: 170, card_width: 480 } }));
    // Sprite hangs below a default card: 720 wide, 170 + 430 tall.
    await expect.poll(size, { timeout: 10_000 }).toEqual([720, 600]);
    await show();
    await expect.poll(async () => {
      const g = await geometry(popupPage());
      return g.sprite && [g.sprite.left, g.sprite.top, g.card.left, g.card.top, g.card.width];
    }).toEqual([0, 170, 0, 0, 480]);
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
