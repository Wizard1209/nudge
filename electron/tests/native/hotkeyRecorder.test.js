import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, openSettingsWindow } from './fixtures/electron.js';

// The recorder in the real Settings window: real key events into a native
// window, and a real window blur — the two things the headless browser tier
// cannot produce (docs/TASK-hotkey-native.md: a test is evidence only if its
// input is a real keypress).
//
// The spelling is this platform's (§5), and these tests run on the host, so
// they expect the host's.
const MAC = process.platform === 'darwin';
const spell = (mac, win) => (MAC ? mac : win);

describe('settings window — recording a hotkey from the keyboard', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function openSettings() {
    ({ app } = await launchApp());
    const settings = await openSettingsWindow(app);
    await settings.bringToFront();
    return settings;
  }

  const field = (page) => page.inputValue('[data-testid="settings-hotkey"]');
  const state = (page) =>
    page.getAttribute('[data-testid="settings-hotkey-record"]', 'data-recorder');

  async function arm(page) {
    await page.click('[data-testid="settings-hotkey-record"]');
    expect(await state(page)).toBe('armed');
  }

  it('records a chord pressed on the keyboard', async () => {
    const settings = await openSettings();
    await arm(settings);
    await settings.keyboard.down('Alt');
    await settings.keyboard.press('KeyJ');
    await settings.keyboard.up('Alt');
    expect(await field(settings)).toBe(spell('⌥J', 'Alt+J'));
    expect(await state(settings)).toBe('idle');
  });

  it('refuses the keypad by name and stays armed', async () => {
    const settings = await openSettings();
    await arm(settings);
    await settings.keyboard.down('Alt');
    await settings.keyboard.press('Numpad5');
    await settings.keyboard.up('Alt');
    expect(await settings.textContent('[data-testid="settings-banner"]')).toContain(
      'Keypad 5',
    );
    expect(await state(settings)).toBe('armed');
  });

  it('puts the saved hotkey back on bare Escape', async () => {
    const settings = await openSettings();
    const before = await field(settings);
    await arm(settings);
    await settings.keyboard.press('Escape');
    expect(await state(settings)).toBe('idle');
    expect(await field(settings)).toBe(before);
  });

  it("cancels on the window's own blur, over IPC", async () => {
    const settings = await openSettings();
    await arm(settings);
    // The window under this harness never becomes the OS's focused window
    // (`isFocused()` stays false, so `blur()` has nothing to give up and the
    // renderer's own `window` blur is suppressed besides). What is exercised
    // here is everything downstream of the OS: the main process's blur
    // handler, the IPC message, and the recorder's focus-lost cell in the real
    // window. The event source itself is covered by the browser tier, where a
    // real focus change out of the page can be produced.
    await app.evaluate(({ BrowserWindow }) => {
      const win = BrowserWindow.getAllWindows().find((w) =>
        w.getTitle().includes('Settings'),
      );
      win.emit('blur');
    });
    await settings.waitForFunction(
      () =>
        document
          .querySelector('[data-testid="settings-hotkey-record"]')
          .dataset.recorder === 'idle',
      undefined,
      { timeout: 5000 },
    );
    expect(await field(settings)).toBe(spell('⌃⇧Space', 'Ctrl+Shift+Space'));
  });
});
