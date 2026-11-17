import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, popupState, openSettingsWindow, settingsState } from './fixtures/electron.js';

const FIELDS = ['field-doing', 'field-bullshit', 'field-minutes'];

describe('popup — Tab navigation must keep focus inside the window', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function open() {
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForSelector('[data-testid="field-doing"]', { state: 'visible' });
    return popup;
  }

  it('focus never escapes the three fields when tabbing forward past the end', async () => {
    const popup = await open();
    await popup.locator('[data-testid="field-doing"]').focus();
    for (let i = 0; i < 5; i++) {
      await popup.keyboard.press('Tab');
      expect(FIELDS).toContain(await activeTestId(popup));
    }
    expect(await formMounted(popup)).toBe(true);
  });

  it('focus never escapes when Shift+Tab is pressed on the first field', async () => {
    const popup = await open();
    await popup.locator('[data-testid="field-doing"]').focus();
    await popup.keyboard.press('Shift+Tab');
    expect(FIELDS).toContain(await activeTestId(popup));
    expect(await formMounted(popup)).toBe(true);
  });

  it('wraps focus at both edges (Tab on last → first, Shift+Tab on first → last)', async () => {
    const popup = await open();

    await popup.locator('[data-testid="field-minutes"]').focus();
    await popup.keyboard.press('Tab');
    expect(await activeTestId(popup)).toBe('field-doing');

    await popup.locator('[data-testid="field-doing"]').focus();
    await popup.keyboard.press('Shift+Tab');
    expect(await activeTestId(popup)).toBe('field-minutes');
  });
});

describe('popup — the card fills its window (nothing clipped)', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  it('sits flush at the top and shows all three fields within the window', async () => {
    const { launchApp } = await import('./fixtures/electron.js');
    ({ app } = await launchApp());
    const popup = await app.firstWindow();
    await popup.waitForSelector('[data-testid="nudge"]', { state: 'attached' });

    const g = await popup.evaluate(() => {
      const r = (sel) => {
        const b = document.querySelector(sel).getBoundingClientRect();
        return { top: b.top, bottom: b.bottom, height: b.height };
      };
      return {
        innerH: window.innerHeight,
        card: r('[data-testid="nudge"]'),
        fields: ['field-doing', 'field-bullshit', 'field-minutes'].map((id) =>
          r(`[data-testid="${id}"]`),
        ),
      };
    });

    const { contentBounds } = await popupState(app);
    expect([contentBounds.width, contentBounds.height]).toEqual([480, 130]);
    expect(Math.abs(g.card.height - 129.5)).toBeLessThanOrEqual(0.5);
    expect(g.card.top).toBeLessThanOrEqual(4);
    for (const f of g.fields) {
      expect(f.bottom).toBeLessThanOrEqual(g.innerH + 1);
    }
  });
});

describe('popup — the display is the one holding the front window', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  it.skipIf(process.platform !== 'darwin')(
    "goes on the display of the frontmost app's front window — Settings, once it is up",
    async () => {
      ({ app } = await launchApp());
      await openSettingsWindow(app);
      const settings = await settingsState(app);

      const read = () =>
        app.evaluate(({ screen }) => {
          const front = globalThis.__nudgeTest.frontWindowBounds();
          return {
            front,
            chosen: globalThis.__nudgeTest.currentDisplay().id,
            matching: front && screen.getDisplayMatching(front).id,
          };
        });
      let r = await read();
      for (const deadline = Date.now() + 5000; Date.now() < deadline; r = await read()) {
        if (r.front && r.front.x === settings.bounds.x && r.front.y === settings.bounds.y) break;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      expect(r.front).toEqual(settings.bounds);
      expect(r.chosen).toBe(r.matching);
    },
  );
});

describe('popup — a trigger while it is up', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function launchFocused(ctx) {
    ({ app } = await launchApp());
    await expect.poll(async () => (await popupState(app))?.visible).toBe(true);
    const isKey = () =>
      app.evaluate(({ BrowserWindow }) =>
        BrowserWindow.getAllWindows().find((x) => x.getTitle() === 'Nudge').isFocused(),
      );
    for (const deadline = Date.now() + 3000; !(await isKey()) && Date.now() < deadline; ) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    if (!(await isKey())) ctx.skip('the launch popup did not take the keyboard on this host');
    return isKey;
  }

  const nudgeOff = () =>
    app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows().find((x) => x.getTitle() === 'Nudge');
      const b = w.getBounds();
      w.setPosition(b.x + 40, b.y + 60);
      return w.getBounds();
    });

  it('two triggers on a key popup change neither its bounds nor its source', async (ctx) => {
    await launchFocused(ctx);
    const before = await popupState(app);
    const state = () => app.evaluate(() => globalThis.__nudgeTest.getState());
    expect((await state()).lastTriggerSource).toBe('timer');

    const moved = await nudgeOff();
    expect([moved.x, moved.y]).toEqual([before.bounds.x + 40, before.bounds.y + 60]);

    await app.evaluate(() => globalThis.__nudgeTest.show());
    await app.evaluate(() => globalThis.__nudgeTest.show());
    await new Promise((resolve) => setTimeout(resolve, 300));

    const after = await popupState(app);
    expect(after.visible).toBe(true);
    expect(after.bounds).toEqual(moved);
    expect((await state()).lastTriggerSource).toBe('timer');
  });

  it('a trigger on a visible popup that is not key puts it back on its spot and makes it key', async (ctx) => {
    const isKey = await launchFocused(ctx);
    const before = await popupState(app);

    await app.evaluate(({ BrowserWindow }) => {
      const w = BrowserWindow.getAllWindows().find((x) => x.getTitle() === 'Nudge');
      w.webContents.send('nudge:show', { gotFocus: false });
      const other = new BrowserWindow({ width: 200, height: 100, title: 'other', show: false });
      other.show();
      other.focus();
    });
    await expect.poll(isKey).toBe(false);
    const moved = await nudgeOff();
    expect([moved.x, moved.y]).toEqual([before.bounds.x + 40, before.bounds.y + 60]);
    expect((await popupState(app)).visible).toBe(true);

    await app.evaluate(() => globalThis.__nudgeTest.show());
    await expect.poll(async () => (await popupState(app)).bounds).toEqual(before.bounds);
    await expect.poll(isKey).toBe(true);
  });
});

function activeTestId(popup) {
  return popup.evaluate(
    () => document.activeElement?.getAttribute('data-testid') ?? document.activeElement?.tagName,
  );
}

function formMounted(popup) {
  return popup.evaluate(() => !!document.querySelector('[data-testid="nudge"]'));
}
