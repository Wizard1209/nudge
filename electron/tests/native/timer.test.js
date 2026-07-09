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
