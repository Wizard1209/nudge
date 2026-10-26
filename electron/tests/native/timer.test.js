import { describe, it, expect, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';

// "Timer dies after first launch." The first auto-shown popup was opened as
// "manual", and Esc/Switch only arm the timer for a "timer"-sourced popup — so
// closing the very first popup with Esc or by clicking away left the app with
// no timer at all (only Enter happened to start it). Per §4 the first popup
// behaves like a timer fire, so every close path must arm the timer.
describe('timer lifecycle — survives the first launch', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function firstPopup() {
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForSelector('[data-testid="field-doing"]', { state: 'visible' });
    await popup.locator('[data-testid="field-doing"]').focus();
    return popup;
  }

  const timerArmed = () =>
    app.evaluate(() => globalThis.__nudgeTest.getState().timerArmed);

  it('Esc on the first popup arms the timer', async () => {
    const popup = await firstPopup();
    await popup.keyboard.press('Escape');
    await popup.waitForTimeout(200);
    expect(await timerArmed()).toBe(true);
  });

  it('Switch (focus loss) on the first popup arms the timer', async () => {
    const popup = await firstPopup();
    // §4 gates Switch on the popup having actually taken focus when it opened,
    // and a window under this harness never becomes the OS's focused one — so
    // the main process reported `gotFocus: false` and the blur below was
    // correctly ignored. Send the payload a focused open produces, then blur.
    await app.evaluate(({ BrowserWindow }) => {
      const win = BrowserWindow.getAllWindows().find((w) => w.getTitle() === 'Nudge');
      win.webContents.send('nudge:show', { gotFocus: true });
    });
    await popup.waitForTimeout(100);
    await popup.evaluate(() => window.dispatchEvent(new Event('blur')));
    await popup.waitForTimeout(200);
    expect(await timerArmed()).toBe(true);
  });

  it('Enter on the first popup arms the timer', async () => {
    const popup = await firstPopup();
    await popup.keyboard.press('Enter');
    await popup.waitForTimeout(200);
    expect(await timerArmed()).toBe(true);
  });
});

// Spec §4: "Deadline cannot arrive while the popup is visible — the timer
// parks for the duration of the show, so an interval that elapses behind an
// open popup is still overdue when it closes and fires then." A manual open
// leaves the timer running behind the card; when it comes due there, the
// nudge it owes is delivered the moment the card closes without restarting
// the timer, and that nudge is the timer's — its Esc re-arms the timer.
describe('timer lifecycle — a deadline behind a manual popup fires when it closes', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  it('Esc on the manual popup is followed at once by the timer popup', async () => {
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForSelector('[data-testid="field-doing"]', { state: 'visible' });
    const state = () => app.evaluate(() => globalThis.__nudgeTest.getState());
    const dismiss = () => popup.evaluate(() => window.nudge.dismiss({ nextMinutes: 0.02 }));

    // The launch popup is the timer's: Esc with 0.02 min arms a 1.2 s deadline
    // (plus the petal fade).
    await dismiss();
    await expect.poll(async () => (await state()).popupVisible).toBe(false);
    await app.evaluate(() => globalThis.__nudgeTest.show());
    expect(await state()).toMatchObject({ lastTriggerSource: 'manual', popupVisible: true });

    // The deadline passes behind the manual card: nothing changes under the
    // user's hands.
    await popup.waitForTimeout(2500);
    expect(await state()).toMatchObject({ lastTriggerSource: 'manual', popupVisible: true });

    // Leave from Asking·Manual leaves the timer alone — and the overdue
    // Deadline follows at once.
    await dismiss();
    await expect.poll(async () => await state(), { timeout: 2000 }).toMatchObject({
      lastTriggerSource: 'timer',
      popupVisible: true,
    });

    // That popup is the timer's: closing it re-arms the timer and it stays
    // closed.
    await dismiss();
    await expect.poll(async () => (await state()).popupVisible).toBe(false);
    expect((await state()).timerArmed).toBe(true);
    await popup.waitForTimeout(500);
    expect((await state()).popupVisible).toBe(false);
  });
});
