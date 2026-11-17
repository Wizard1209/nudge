import { describe, it, expect, afterEach } from 'vitest';
import { readFileSync } from 'node:fs';
import { launchApp, settingsState, openSettingsWindow } from './fixtures/electron.js';

describe('settings window — no gray flash, content fully painted', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function openSettings() {
    ({ app } = await launchApp());
    return openSettingsWindow(app);
  }

  it('window has an opaque dark backgroundColor (no white/gray flash)', async () => {
    await openSettings();
    const state = await settingsState(app);
    expect(state).not.toBeNull();
    expect(state.backgroundColor.toUpperCase()).not.toBe('#FFFFFF');
    expect(state.backgroundColor.toUpperCase()).not.toBe('#FFF');
  });

  it('the page background is opaque (does not reveal the window chrome)', async () => {
    const settings = await openSettings();
    const bg = await settings.evaluate(() => {
      const body = getComputedStyle(document.body).backgroundColor;
      const html = getComputedStyle(document.documentElement).backgroundColor;
      return { body, html };
    });
    const transparent = (c) => c === 'rgba(0, 0, 0, 0)' || c === 'transparent';
    expect(transparent(bg.body) && transparent(bg.html)).toBe(false);
  });

  it('the window is sized to the form (nothing clipped, nothing to spare)', async () => {
    const settings = await openSettings();
    const fit = await settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      formBottom: document.querySelector('[data-testid="settings-save"]').getBoundingClientRect().bottom,
    }));
    expect(fit.scroll).toBe(fit.inner);
    expect(fit.formBottom).toBeCloseTo(fit.inner - 20, 0);
  });

  it('a refused save neither moves the buttons nor scrolls the form', async () => {
    const settings = await openSettings();
    const probe = () => settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      saveTop: document.querySelector('[data-testid="settings-save"]').getBoundingClientRect().top,
    }));
    const quiet = await probe();
    await settings.fill('[data-testid="settings-hotkey"]', 'J');
    await settings.click('[data-testid="settings-save"]');
    await settings.waitForSelector('[data-testid="settings-banner"]');
    expect(await probe()).toEqual(quiet);
    expect(quiet.scroll).toBe(quiet.inner);
  });

  it('all controls are present and visible', async () => {
    const settings = await openSettings();
    for (const id of [
      'settings-hotkey',
      'settings-hotkey-record',
      'settings-interval',
      'settings-autostart',
      'settings-sprite-enabled',
      'settings-sprite-path',
      'settings-sprite-browse',
      'settings-sprite-x',
      'settings-sprite-y',
      'settings-card-width',
      'settings-update',
      'settings-save',
      'settings-cancel',
    ]) {
      expect(await settings.locator(`[data-testid="${id}"]`).isVisible()).toBe(true);
    }
  });
});

describe('settings window — §9 layout', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  async function openSettings() {
    ({ app } = await launchApp());
    return openSettingsWindow(app);
  }

  const AUTOSTART = process.platform === 'darwin' ? 'Open at login' : 'Launch with Windows';
  const VERSION = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url))).version;

  function readGroups(settings) {
    return settings.evaluate(() => {
      const box = (el) => {
        const r = el.getBoundingClientRect();
        return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height };
      };
      const groups = [...document.querySelectorAll('main [class*="rounded-[10px]"]')];
      return groups.map((g) => {
        const style = getComputedStyle(g);
        return {
          box: box(g),
          radius: style.borderRadius,
          borderWidth: style.borderTopWidth,
          rows: [...g.children].map((row) => {
            const rs = getComputedStyle(row);
            const label = row.firstElementChild;
            const control = row.lastElementChild;
            return {
              text: label.textContent.trim(),
              box: box(row),
              innerLeft: row.getBoundingClientRect().left + parseFloat(rs.paddingLeft),
              innerRight: row.getBoundingClientRect().right - parseFloat(rs.paddingRight),
              label: box(label),
              control: box(control),
              controlTag: control.tagName.toLowerCase(),
              controlType: control.getAttribute('type'),
              borderTop: parseFloat(rs.borderTopWidth),
              borderBottom: parseFloat(rs.borderBottomWidth),
            };
          }),
        };
      });
    });
  }

  it('is four groups of rows in rounded boxes, a hairline between rows', async () => {
    const settings = await openSettings();
    const groups = await readGroups(settings);
    expect(groups.map((g) => g.rows.map((r) => r.text))).toEqual([
      ['Global hotkey'],
      ['Default interval', AUTOSTART],
      ['Sprite', 'PNG', 'Position (pt)'],
      [`Nudge ${VERSION}`],
    ]);
    expect(groups[0].box.bottom).toBeLessThan(groups[1].box.top);
    expect(groups[1].box.bottom).toBeLessThan(groups[2].box.top);
    expect(groups[2].box.bottom).toBeLessThan(groups[3].box.top);
    for (const g of groups) {
      expect(parseFloat(g.radius)).toBeGreaterThan(0);
      expect(parseFloat(g.borderWidth)).toBeGreaterThan(0);
      for (let i = 1; i < g.rows.length; i += 1) {
        expect(g.rows[i - 1].borderBottom > 0 || g.rows[i].borderTop > 0, `seam ${i}`).toBe(true);
      }
    }
  });

  it('puts the label at the left and the control at the right edge of every row', async () => {
    const settings = await openSettings();
    const rows = (await readGroups(settings)).flatMap((g) => g.rows);
    expect(rows).toHaveLength(7);
    for (const row of rows) {
      expect(row.label.left, `${row.text}: label left`).toBeCloseTo(row.innerLeft, 0);
      expect(row.control.right, `${row.text}: control right`).toBeCloseTo(row.innerRight, 0);
      expect(row.label.right, `${row.text}: label before control`).toBeLessThan(row.control.left);
    }
    const lefts = new Set(rows.map((r) => Math.round(r.label.left)));
    const rights = new Set(rows.map((r) => Math.round(r.control.right)));
    expect([...lefts]).toHaveLength(1);
    expect([...rights]).toHaveLength(1);
  });

  it('has the hotkey vocabulary as a footnote under the first group', async () => {
    const settings = await openSettings();
    const groups = await readGroups(settings);
    const note = await settings.locator('[data-testid="settings-hotkey-vocabulary"]').boundingBox();
    const noteTop = note.y;
    const noteBottom = note.y + note.height;
    expect(noteTop).toBeGreaterThanOrEqual(groups[0].box.bottom);
    expect(noteBottom).toBeLessThanOrEqual(groups[1].box.top);
    expect(noteTop - groups[0].box.bottom).toBeLessThan(groups[1].box.top - noteBottom);
  });

  it('puts each checkbox at the right of its row, after the row text', async () => {
    const settings = await openSettings();
    const rows = (await readGroups(settings)).flatMap((g) => g.rows);
    const boxes = rows.filter((r) => r.controlType === 'checkbox');
    expect(boxes.map((r) => r.text)).toEqual([AUTOSTART, 'Sprite']);
    for (const row of boxes) {
      expect(row.control.right).toBeCloseTo(row.innerRight, 0);
      expect(row.control.left).toBeGreaterThan(row.label.right);
    }
    const before = await settings.locator('[data-testid="settings-sprite-enabled"]').isChecked();
    await settings.getByText('Sprite', { exact: true }).click();
    expect(await settings.locator('[data-testid="settings-sprite-enabled"]').isChecked()).toBe(!before);
  });

  it('has no heading inside the window', async () => {
    const settings = await openSettings();
    const title = (await settingsState(app)).title;
    const inside = await settings.evaluate(() => ({
      headings: document.querySelectorAll('h1, h2, h3, h4, h5, h6, [role="heading"]').length,
      lines: document.body.innerText.split('\n').map((l) => l.trim()).filter(Boolean),
    }));
    expect(inside.headings).toBe(0);
    expect(inside.lines).not.toContain(title);
    expect(inside.lines).not.toContain('Settings');
  });

  it('is sized to the form and not resizable', async () => {
    const settings = await openSettings();
    const resizable = await app.evaluate(({ BrowserWindow }) =>
      BrowserWindow.getAllWindows().find((w) => w.getTitle().includes('Settings')).isResizable());
    expect(resizable).toBe(false);
    const fit = await settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
    }));
    expect(fit.scroll).toBe(fit.inner);
    expect((await settingsState(app)).contentBounds.height).toBe(fit.inner);
  });

  it('keeps the banner line and the window size when a save is refused', async () => {
    const settings = await openSettings();
    const line = () => settings.evaluate(() => {
      const buttons = document.querySelector('[data-testid="settings-save"]').parentElement;
      const l = buttons.previousElementSibling.getBoundingClientRect();
      const b = buttons.getBoundingClientRect();
      return { top: l.top, height: l.height, buttonsTop: b.top };
    });
    const quiet = await line();
    const quietWindow = (await settingsState(app)).contentBounds;
    expect(await settings.locator('[data-testid="settings-banner"]').count()).toBe(0);
    expect(quiet.height).toBeGreaterThan(0);

    await settings.fill('[data-testid="settings-interval"]', 'x');
    await settings.click('[data-testid="settings-save"]');
    await settings.waitForSelector('[data-testid="settings-banner"]');
    const banner = await settings.locator('[data-testid="settings-banner"]').boundingBox();

    const spoken = await line();
    expect(spoken).toEqual(quiet);
    expect(banner.y).toBeGreaterThanOrEqual(quiet.top);
    expect(banner.y + banner.height).toBeLessThanOrEqual(quiet.buttonsTop);
    expect((await settingsState(app)).contentBounds).toEqual(quietWindow);
  });

  it('puts the Saved note to the left of the buttons after a save', async () => {
    const settings = await openSettings();
    expect(await settings.locator('[data-testid="settings-saved"]').count()).toBe(0);
    await settings.fill('[data-testid="settings-interval"]', '5');
    await settings.click('[data-testid="settings-save"]');
    await settings.waitForSelector('[data-testid="settings-saved"]');
    const note = await settings.locator('[data-testid="settings-saved"]').boundingBox();
    const cancel = await settings.locator('[data-testid="settings-cancel"]').boundingBox();
    expect(note.x + note.width).toBeLessThanOrEqual(cancel.x);
    expect(note.y).toBeLessThan(cancel.y + cancel.height);
    expect(cancel.y).toBeLessThan(note.y + note.height);
  });

  it('keeps the banner on its line when the refused hotkey is a long token', async () => {
    const settings = await openSettings();
    const probe = () => settings.evaluate(() => {
      const buttons = document.querySelector('[data-testid="settings-save"]').parentElement;
      const l = buttons.previousElementSibling.getBoundingClientRect();
      const b = buttons.getBoundingClientRect();
      return {
        scroll: document.documentElement.scrollHeight,
        inner: window.innerHeight,
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
        line: { top: l.top, bottom: l.bottom, left: l.left, right: l.right },
        buttonsTop: b.top,
      };
    });
    const quiet = await probe();
    for (const token of ['abcdefghij'.repeat(15), 'abcde '.repeat(25)]) {
      await settings.fill('[data-testid="settings-hotkey"]', token);
      await settings.click('[data-testid="settings-save"]');
      await settings.waitForSelector('[data-testid="settings-banner"]');
      const spoken = await probe();
      expect(spoken, token).toEqual(quiet);
      expect(spoken.scroll, token).toBe(spoken.inner);
      expect(spoken.scrollWidth, token).toBe(spoken.innerWidth);
      const banner = await settings.locator('[data-testid="settings-banner"]').boundingBox();
      expect(banner.y, token).toBeGreaterThanOrEqual(quiet.line.top);
      expect(banner.y + banner.height, token).toBeLessThanOrEqual(quiet.line.bottom);
      expect(banner.x, token).toBeGreaterThanOrEqual(quiet.line.left);
      expect(banner.x + banner.width, token).toBeLessThanOrEqual(quiet.line.right);
    }
  });

  it('has Cancel then Save at the bottom right, Save the one filled button', async () => {
    const settings = await openSettings();
    const cancel = await settings.locator('[data-testid="settings-cancel"]').boundingBox();
    const save = await settings.locator('[data-testid="settings-save"]').boundingBox();
    const groups = await readGroups(settings);
    const { innerHeight, main } = await settings.evaluate(() => {
      const s = getComputedStyle(document.querySelector('main'));
      return { innerHeight: window.innerHeight, main: { bottom: parseFloat(s.paddingBottom) } };
    });
    expect(cancel.x + cancel.width).toBeLessThanOrEqual(save.x);
    expect(Math.abs(cancel.y - save.y)).toBeLessThan(1);
    expect(save.x + save.width).toBeCloseTo(groups[2].box.right, 0);
    expect(save.y + save.height).toBeCloseTo(innerHeight - main.bottom, 0);
    const fills = await settings.evaluate(() => {
      const bg = (id) => getComputedStyle(document.querySelector(`[data-testid="${id}"]`)).backgroundColor;
      return { save: bg('settings-save'), cancel: bg('settings-cancel') };
    });
    expect(fills.save).not.toBe('rgba(0, 0, 0, 0)');
    expect(fills.save).not.toBe(fills.cancel);
  });
});

describe('settings window — §9 Updates', () => {
  let app;
  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  const release = (tag) => ({
    tag_name: tag,
    html_url: `https://github.com/Wizard1209/nudge/releases/tag/${tag}`,
    assets: [],
  });

  async function openSettings() {
    ({ app } = await launchApp());
    return openSettingsWindow(app);
  }

  const status = (settings) =>
    settings.locator('[data-testid="settings-update-status"]').innerText();
  const button = (settings) => settings.locator('[data-testid="settings-update"]');

  it('asks nothing until pressed, and says where it would look', async () => {
    const settings = await openSettings();
    expect(await status(settings)).toBe('');
    expect(await button(settings).innerText()).toBe('Check for updates');
    expect(await settings.locator('[data-testid="settings-update-footnote"]').innerText()).toBe(
      'Checks github.com/Wizard1209/nudge for new releases, only when pressed.',
    );
  });

  it('a newer release offers its page, and the button opens it', async () => {
    const settings = await openSettings();
    await app.evaluate((_e, r) => globalThis.__nudgeTest.queueCheck({ ok: true, release: r }), release('v99.0.0'));
    await button(settings).click();
    await settings.waitForFunction(() =>
      document.querySelector('[data-testid="settings-update-status"]').innerText === '99.0.0 available');
    expect(await button(settings).innerText()).toBe('Open release page');
    await button(settings).click();
    await settings.waitForTimeout(200);
    expect(await app.evaluate(() => globalThis.__nudgeTest.openedReleases())).toEqual([
      'https://github.com/Wizard1209/nudge/releases/tag/v99.0.0',
    ]);
  });

  it('an older or equal release is up to date', async () => {
    const settings = await openSettings();
    await app.evaluate((_e, r) => globalThis.__nudgeTest.queueCheck({ ok: true, release: r }), release('v0.0.1'));
    await button(settings).click();
    await settings.waitForFunction(() =>
      document.querySelector('[data-testid="settings-update-status"]').innerText === 'Up to date');
    expect(await button(settings).innerText()).toBe('Check for updates');
  });

  it('a failed check shows its reason', async () => {
    const settings = await openSettings();
    await app.evaluate(() =>
      globalThis.__nudgeTest.queueCheck({ ok: false, error: 'Offline: could not reach GitHub' }));
    await button(settings).click();
    await settings.waitForFunction(() =>
      document.querySelector('[data-testid="settings-update-status"]').innerText ===
      'Offline: could not reach GitHub');
  });
});
