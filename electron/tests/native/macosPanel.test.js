import { describe, it, expect, beforeAll, beforeEach, afterEach } from 'vitest';
import { launchApp } from './fixtures/electron.js';
import { macosInput, KEY } from './fixtures/macosInput.js';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

describe.skipIf(process.platform !== 'darwin')('macOS popup window', () => {
  let input;
  let app;
  beforeAll(() => {
    input = macosInput();
  });
  beforeEach((ctx) => {
    if (input.locked()) ctx.skip('the screen is locked');
  });
  afterEach(async () => {
    const proc = app?.process();
    await Promise.race([app?.close().catch(() => {}), sleep(5000)]);
    if (proc && proc.exitCode === null && proc.signalCode === null) proc.kill('SIGKILL');
    app = undefined;
  });

  const native = () =>
    app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows().find((x) => x.getTitle() === 'Nudge');
      return { visible: w.isVisible(), focused: w.isFocused(), bounds: w.getBounds() };
    });
  const display = () => app.evaluate(({ screen }) => screen.getPrimaryDisplay());
  const workArea = async () => (await display()).workArea;
  const state = () => app.evaluate(() => globalThis.__nudgeTest.getState());

  async function waitFor(pred, ms, step = 20) {
    const deadline = Date.now() + ms;
    for (;;) {
      const v = await pred();
      if (v || Date.now() > deadline) return v;
      await sleep(step);
    }
  }

  const inside = (b, wa) =>
    b.x >= wa.x && b.y >= wa.y && b.x + b.width <= wa.x + wa.width && b.y + b.height <= wa.y + wa.height;
  const contains = (w, p) => p.x >= w.x && p.x < w.x + w.width && p.y >= w.y && p.y < w.y + w.height;

  it('after a wallpaper click, re-show puts the card back on screen and key', async (ctx) => {
    ({ app } = await launchApp());
    expect(await waitFor(async () => (await native()).visible, 10_000)).toBe(true);
    const wa = await workArea();
    const shown = await native();

    const probe0 = await app.evaluate(({ BrowserWindow }, wa) => {
      const p = new BrowserWindow({
        x: wa.x + wa.width - 260, y: wa.y + wa.height - 160, width: 220, height: 120,
        show: false, title: 'probe', backgroundColor: '#808080',
      });
      p.showInactive();
      globalThis.__probe = p;
      globalThis.__blurs = 0;
      BrowserWindow.getAllWindows().find((x) => x.getTitle() === 'Nudge')
        .on('blur', () => globalThis.__blurs++);
      return p.getBounds();
    }, wa);
    const probe = () => app.evaluate(() => globalThis.__probe.getBounds());
    const probeHome = async () => {
      const b = await probe();
      return b.x === probe0.x && b.y === probe0.y;
    };

    const wall = { x: wa.x + 10, y: Math.round(wa.y + wa.height / 2) };
    const onScreen = input.windows();
    if (onScreen.some((w) => w.name === 'Fullscreen Backdrop')) {
      ctx.skip('the current Space is a full-screen app; switch to a desktop Space');
    }
    const covering = onScreen.filter((w) => w.layer >= 0 && w.layer < 20 && contains(w, wall));
    if (covering.length) {
      ctx.skip(`wallpaper point ${wall.x},${wall.y} is covered by ${covering.map((w) => `${w.owner} "${w.name}"`).join(', ')}`);
    }

    try {
      input.click(wall.x, wall.y);
      let left = false;
      let hidden = false;
      let swept = false;
      let minY = shown.bounds.y;
      const t0 = Date.now();
      while (Date.now() - t0 < 2000) {
        const n = await native();
        minY = Math.min(minY, n.bounds.y);
        left ||= !inside(n.bounds, wa);
        hidden ||= !n.visible;
        swept ||= !(await probeHome());
        await sleep(25);
      }
      const blurs = await app.evaluate(() => globalThis.__blurs);
      console.log('[wallpaper click]', JSON.stringify({ shownY: shown.bounds.y, minY, left, hidden, blurs, probeSwept: swept, probeNow: await probe() }));
      if (!(swept || left)) ctx.skip('a wallpaper click moved no window: this host does not reveal the desktop');

      input.key(KEY.space, 'ctrl', 'shift');
      await waitFor(async () => (await native()).visible, 2000);
      await sleep(500);
      const after = await native();
      console.log('[after hotkey]', JSON.stringify(after), 'probe home:', await probeHome());
      expect(after.visible).toBe(true);
      expect(inside(after.bounds, wa), `bounds ${JSON.stringify(after.bounds)} outside workArea ${JSON.stringify(wa)}`).toBe(true);
      const { bounds: mon } = await display();
      expect(after.bounds.y).toBe(Math.round(mon.y + mon.height * 0.25) + 80);
      expect(after.bounds.x).toBe(Math.round(mon.x + (mon.width - after.bounds.width) / 2));
      expect(after.focused).toBe(true);
    } finally {
      if (!(await waitFor(probeHome, 1000))) input.click(wall.x, wall.y);
      expect(await waitFor(probeHome, 3000), 'the desktop is still in reveal mode').toBe(true);
    }
  });

  it('a popup that appears while another app keeps grabbing activation is still key, and raw Esc closes it', async () => {
    const stop = input.contend('com.apple.finder', 30);
    try {
      ({ app } = await launchApp());
      expect(await waitFor(async () => (await native()).visible, 10_000, 10)).toBe(true);
      await sleep(300);
    } finally {
      await stop();
    }
    await sleep(1000);
    const pid = app.process().pid;
    const n = await native();
    const front = input.frontPid();
    console.log('[after contention]', JSON.stringify({ focused: n.focused, frontPid: front, electronPid: pid }));
    expect(n.focused, 'the popup is not the key window').toBe(true);
    expect(front, 'the app was activated to get the keyboard').not.toBe(pid);

    input.key(KEY.esc);
    expect(await waitFor(async () => !(await native()).visible, 1000), 'Esc did not close the popup').toBe(true);
    expect((await state()).timerArmed).toBe(true);
  });
});
