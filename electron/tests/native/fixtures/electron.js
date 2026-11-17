import { _electron as electron } from 'playwright';
import path from 'node:path';
import { mkdtempSync } from 'node:fs';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const MAIN = path.join(root, 'dist', 'electron', 'main.js');

export async function launchApp(extraEnv = {}) {
  const app = await electron.launch({
    args: [MAIN],
    env: {
      ...process.env,
      NUDGE_E2E: '1',
      NUDGE_FORCE_PROD: '1',
      NUDGE_DIR: mkdtempSync(path.join(os.tmpdir(), 'nudge-native-')),
      ...extraEnv,
    },
  });
  try {
    const popup = await app.firstWindow();
    await popup.waitForLoadState('domcontentloaded');
    return { app, popup };
  } catch (err) {
    app.process().kill('SIGKILL');
    throw err;
  }
}

export function windowStates(app) {
  return app.evaluate(({ BrowserWindow }) =>
    BrowserWindow.getAllWindows().map((w) => ({
      title: w.getTitle(),
      visible: w.isVisible(),
      bounds: w.getBounds(),
      contentBounds: w.getContentBounds(),
      backgroundColor: w.getBackgroundColor(),
    })),
  );
}

export async function popupState(app) {
  const all = await windowStates(app);
  return all.find((w) => w.title === 'Nudge') ?? null;
}

export async function settingsState(app) {
  const all = await windowStates(app);
  return all.find((w) => w.title.includes('Settings')) ?? null;
}

export async function openSettingsWindow(app) {
  const before = app.windows().length;
  await app.evaluate(() => globalThis.__nudgeTest.openSettings());
  const deadline = Date.now() + 15_000;
  let page;
  while (Date.now() < deadline) {
    page = app.windows().find((p) => /settings\.html/.test(p.url()));
    if (page) break;
    if (app.windows().length > before) {
      page = app.windows()[app.windows().length - 1];
      if (/settings/.test(page.url())) break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  if (!page) throw new Error('Settings window did not open');
  await page.waitForLoadState('domcontentloaded');
  await page.waitForSelector('[data-testid="settings-save"]', { state: 'visible' });
  return page;
}
