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

  const waitIdle = (page) =>
    page.waitForFunction(
      () =>
        document
          .querySelector('[data-testid="settings-hotkey-record"]')
          .dataset.recorder === 'idle',
      undefined,
      { timeout: 5000 },
    );

  it("cancels on the window's own blur, over IPC", async () => {
    const settings = await openSettings();
    await arm(settings);
    // Everything downstream of the OS, with the event source stubbed: the main
    // process's blur handler, the IPC message, and the recorder's focus-lost
    // cell in the real window. It runs on any session, including one where no
    // window can be focused at all. The OS half — a focus change the system
    // itself delivers — is the test below.
    await app.evaluate(({ BrowserWindow }) => {
      const win = BrowserWindow.getAllWindows().find((w) =>
        w.getTitle().includes('Settings'),
      );
      win.emit('blur');
    });
    await waitIdle(settings);
    expect(await field(settings)).toBe(spell('⌃⇧Space', 'Ctrl+Shift+Space'));
  });

  it('cancels when the OS hands the focus to another window', async (ctx) => {
    const settings = await openSettings();
    // Ask the system for the focus and check we actually got it. On a session
    // that will not activate the app — a locked screen, most CI — no window is
    // focused, so there is no focus to lose and this event cannot be produced
    // at all. Skipping says that; faking the blur here would only re-run the
    // test above under a name that claims more than it did.
    const focused = await app.evaluate(async ({ app: electronApp, BrowserWindow }) => {
      const win = BrowserWindow.getAllWindows().find((w) =>
        w.getTitle().includes('Settings'),
      );
      electronApp.focus({ steal: true });
      win.show();
      win.focus();
      await new Promise((r) => setTimeout(r, 500));
      return win.isFocused();
    });
    if (!focused) {
      ctx.skip(
        'no window can hold the focus in this session (locked screen / non-activating harness)',
      );
    }
    await arm(settings);
    // A second real window takes the focus, and the OS delivers the blur.
    await app.evaluate(({ BrowserWindow }) => {
      const other = new BrowserWindow({ width: 320, height: 200, title: 'Focus thief' });
      other.loadURL('data:text/html,<title>Focus thief</title><h1>thief</h1>');
      other.show();
      other.focus();
    });
    await waitIdle(settings);
    expect(await field(settings)).toBe(spell('⌃⇧Space', 'Ctrl+Shift+Space'));
  });
});
