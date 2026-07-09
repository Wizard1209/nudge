import { _electron as electron } from 'playwright';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const MAIN = path.join(root, 'dist', 'electron', 'main.js');

// These tests drive the REAL Electron app (main process + native windows) —
// the layer the browser/Puppeteer suites can't reach. They need a display
// (WSLg / X11) and must run UNSANDBOXED so Electron can open the X socket.
//
// NUDGE_FORCE_PROD makes an unpackaged build load the *shipped* renderer files
// (dist/renderer/*.html) instead of a Vite dev server — so a test exercises the
// exact assets the portable .exe ships. NUDGE_E2E installs a tiny main-process
// test hook (globalThis.__nudgeTest) used to open Settings / probe the tray.
export async function launchApp(extraEnv = {}) {
  const app = await electron.launch({
    args: [MAIN],
    env: {
      ...process.env,
      NUDGE_E2E: '1',
      NUDGE_FORCE_PROD: '1',
      ...extraEnv,
    },
  });
  const popup = await app.firstWindow();
  await popup.waitForLoadState('domcontentloaded');
  return { app, popup };
}

/** Snapshot of every BrowserWindow's native state, keyed for assertions. */
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

/** The popup window's native state (title "Nudge"). */
export async function popupState(app) {
  const all = await windowStates(app);
  return all.find((w) => w.title === 'Nudge') ?? null;
}

/** The settings window's native state (title contains "Settings"). */
export async function settingsState(app) {
  const all = await windowStates(app);
  return all.find((w) => w.title.includes('Settings')) ?? null;
}

/**
 * Open the Settings window via the main-process test hook and return its Page,
 * fully loaded. Polls app.windows() rather than waitForEvent('window') — the
 * latter races the (synchronous) window creation and can miss the event.
 */
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
