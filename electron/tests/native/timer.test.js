import { describe, it, expect, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';

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

    await dismiss();
    await expect.poll(async () => (await state()).popupVisible).toBe(false);
    await app.evaluate(() => globalThis.__nudgeTest.show());
    expect(await state()).toMatchObject({ lastTriggerSource: 'manual', popupVisible: true });

    await popup.waitForTimeout(2500);
    expect(await state()).toMatchObject({ lastTriggerSource: 'manual', popupVisible: true });

    await dismiss();
    await expect.poll(async () => await state(), { timeout: 2000 }).toMatchObject({
      lastTriggerSource: 'timer',
      popupVisible: true,
    });

    await dismiss();
    await expect.poll(async () => (await state()).popupVisible).toBe(false);
    expect((await state()).timerArmed).toBe(true);
    await popup.waitForTimeout(500);
    expect((await state()).popupVisible).toBe(false);
  });
});
