import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { launchApp, popupState } from './fixtures/electron.js';

const FIXTURE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../e2e/fixtures/sprite-green.png');
const REAL = { enabled: true, path: FIXTURE, x: -41, y: -277, card_width: 679 };

// Spec §2: on macOS the frost behind the card is the system's own material,
// an NSVisualEffectView clipped to the card and sitting under the page. The
// page cannot see it, so these ask the main process where it put the view,
// in the page's own coordinates (origin top-left of the window's content).
// The glass is the card's 129.5 pt (§2), not the 130 pt window row that
// holds it: the half point under the bottom hairline stays bare.
describe.skipIf(process.platform !== 'darwin')('card backdrop (macOS glass)', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function launch(config) {
    const dir = mkdtempSync(path.join(os.tmpdir(), 'nudge-native-'));
    if (config) writeFileSync(path.join(dir, 'config.json'), JSON.stringify(config));
    let popup;
    ({ app, popup } = await launchApp({ NUDGE_DIR: dir }));
    await expect.poll(async () => (await popupState(app))?.visible).toBe(true);
    return { popup, dir };
  }

  const backdrop = () => app.evaluate(() => globalThis.__nudgeTest.backdrop());

  function popupPage() {
    return app.windows().find((p) => /index\.html/.test(p.url()) && !p.isClosed());
  }

  async function hide(page) {
    await page.keyboard.press('Escape');
    await expect.poll(async () => (await popupState(app))?.visible).toBe(false);
  }

  async function show() {
    await app.evaluate(() => globalThis.__nudgeTest.show());
    await expect.poll(async () => (await popupState(app))?.visible).toBe(true);
  }

  it('loads the addon inside Electron and clips the glass to the plain card', async () => {
    await launch(null);
    expect(await backdrop()).toMatchObject({
      loaded: true,
      hidden: false,
      frame: { x: 0, y: 0, width: 480, height: 129.5 },
      radius: 16,
    });
  });

  it('puts the glass under the card, not under the sprite', async () => {
    await launch({ sprite: REAL });
    expect(await backdrop()).toMatchObject({
      hidden: false,
      frame: { x: 41, y: 277, width: 679, height: 129.5 },
    });
  });

  it('takes the glass away with the popup and brings it back on show', async () => {
    const { popup } = await launch(null);
    await hide(popup);
    expect((await backdrop()).hidden).toBe(true);
    await show();
    expect((await backdrop()).hidden).toBe(false);
  });

  it('follows the card when the sprite moves it', async () => {
    const { popup, dir } = await launch({ sprite: REAL });
    await hide(popup);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, x: 0, y: 130, card_width: 480 } }));
    await expect.poll(async () => (await backdrop()).frame, { timeout: 10_000 }).toEqual({ x: 0, y: 0, width: 480, height: 129.5 });
    await show();
    expect(await backdrop()).toMatchObject({ hidden: false, frame: { x: 0, y: 0, width: 480, height: 129.5 } });
  });

  it('survives the window being recreated when the sprite is turned off', async () => {
    const { popup, dir } = await launch({ sprite: REAL });
    await hide(popup);
    writeFileSync(path.join(dir, 'config.json'), JSON.stringify({ sprite: { ...REAL, enabled: false } }));
    await expect.poll(async () => (await popupState(app))?.contentBounds.height, { timeout: 10_000 }).toBe(130);
    await show();
    expect(await backdrop()).toMatchObject({ hidden: false, frame: { x: 0, y: 0, width: 480, height: 129.5 } });
    expect(await popupPage().$('[data-testid="sprite"]')).toBeNull();
  });
});
