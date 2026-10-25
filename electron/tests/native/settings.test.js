import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, settingsState, openSettingsWindow } from './fixtures/electron.js';

// Bug 1: "everything in a gray frame, and on first open only half is visible."
// Two native defects:
//   (a) the window has no backgroundColor and the renderer's <html>/<body> are
//       transparent → the default white/gray window shows through ("gray frame");
//   (b) the window uses show:true (default) so it appears BEFORE the renderer
//       paints → a gray, half-rendered first frame ("only half is visible").
// The fix: dark window backgroundColor + dark html/body, and show the window
// only on 'ready-to-show'. The frame/title bar itself stays (spec §9).
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
    // Default (unset) is white #FFFFFF. We want the zinc-950 surface.
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
    // A transparent body lets the gray native window show through. At least one
    // of html/body must paint an opaque dark surface.
    const transparent = (c) => c === 'rgba(0, 0, 0, 0)' || c === 'transparent';
    expect(transparent(bg.body) && transparent(bg.html)).toBe(false);
  });

  // §9: the window is sized to the form — nothing clipped, no empty band
  // below the buttons: the button row ends one 20px margin above the bottom.
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

  // §9: the banner keeps its line above the buttons whether or not it has
  // anything to say, so a refused save moves nothing and scrolls nothing.
  it('a refused save neither moves the buttons nor scrolls the form', async () => {
    const settings = await openSettings();
    const probe = () => settings.evaluate(() => ({
      scroll: document.documentElement.scrollHeight,
      inner: window.innerHeight,
      saveTop: document.querySelector('[data-testid="settings-save"]').getBoundingClientRect().top,
    }));
    const quiet = await probe();
    // The longest message the banner carries: a bare key, refused with the
    // vocabulary after it.
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
      'settings-save',
      'settings-cancel',
    ]) {
      expect(await settings.locator(`[data-testid="${id}"]`).isVisible()).toBe(true);
    }
  });
});

// §9 "Window chrome and layout", read bullet by bullet against the real
// window. Each test names the sentence it pins and asserts that sentence
// alone; the geometry comes from boundingBox() / getBoundingClientRect() of
// the elements in the live page, not from the stylesheet.
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

  // The hotkey vocabulary's platform is the host's (§5: one spelling per
  // platform), and so is the autostart row's name (§9).
  const AUTOSTART = process.platform === 'darwin' ? 'Open at login' : 'Launch with Windows';

  // The three rounded boxes, each with its rows: a row's first element is
  // its label and its last element its control. Everything the layout tests
  // measure is read in one pass so the numbers describe one frame.
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
              // The hairline between rows, on whichever side of the seam the
              // stylesheet draws it.
              borderTop: parseFloat(rs.borderTopWidth),
              borderBottom: parseFloat(rs.borderBottomWidth),
            };
          }),
        };
      });
    });
  }

  // "The form is three groups of rows in rounded boxes, a hairline between
  // rows: the hotkey; the interval and autostart; the sprite (checkbox, PNG,
  // position)."
  it('is three groups of rows in rounded boxes, a hairline between rows', async () => {
    const settings = await openSettings();
    const groups = await readGroups(settings);
    expect(groups.map((g) => g.rows.map((r) => r.text))).toEqual([
      ['Global hotkey'],
      ['Default interval', AUTOSTART],
      ['Sprite', 'PNG', 'Position (pt)'],
    ]);
    // In that order, top to bottom.
    expect(groups[0].box.bottom).toBeLessThan(groups[1].box.top);
    expect(groups[1].box.bottom).toBeLessThan(groups[2].box.top);
    for (const g of groups) {
      expect(parseFloat(g.radius)).toBeGreaterThan(0);
      expect(parseFloat(g.borderWidth)).toBeGreaterThan(0);
      // A hairline between rows: every seam has one.
      for (let i = 1; i < g.rows.length; i += 1) {
        expect(g.rows[i - 1].borderBottom > 0 || g.rows[i].borderTop > 0, `seam ${i}`).toBe(true);
      }
    }
  });

  // "In every row the label sits at the left and the control at the right
  // edge."
  it('puts the label at the left and the control at the right edge of every row', async () => {
    const settings = await openSettings();
    const rows = (await readGroups(settings)).flatMap((g) => g.rows);
    expect(rows).toHaveLength(6);
    for (const row of rows) {
      expect(row.label.left, `${row.text}: label left`).toBeCloseTo(row.innerLeft, 0);
      expect(row.control.right, `${row.text}: control right`).toBeCloseTo(row.innerRight, 0);
      expect(row.label.right, `${row.text}: label before control`).toBeLessThan(row.control.left);
    }
    // One edge for all rows, so the groups read as one form.
    const lefts = new Set(rows.map((r) => Math.round(r.label.left)));
    const rights = new Set(rows.map((r) => Math.round(r.control.right)));
    expect([...lefts]).toHaveLength(1);
    expect([...rights]).toHaveLength(1);
  });

  // "The hotkey vocabulary is a footnote under its group."
  it('has the hotkey vocabulary as a footnote under the first group', async () => {
    const settings = await openSettings();
    const groups = await readGroups(settings);
    const note = await settings.locator('[data-testid="settings-hotkey-vocabulary"]').boundingBox();
    const noteTop = note.y;
    const noteBottom = note.y + note.height;
    expect(noteTop).toBeGreaterThanOrEqual(groups[0].box.bottom);
    expect(noteBottom).toBeLessThanOrEqual(groups[1].box.top);
    // A footnote belongs to the box above it: nearer to it than to the next.
    expect(noteTop - groups[0].box.bottom).toBeLessThan(groups[1].box.top - noteBottom);
  });

  // "A checkbox sits at the right of its row; the row's text is its label."
  it('puts each checkbox at the right of its row, after the row text', async () => {
    const settings = await openSettings();
    const rows = (await readGroups(settings)).flatMap((g) => g.rows);
    const boxes = rows.filter((r) => r.controlType === 'checkbox');
    expect(boxes.map((r) => r.text)).toEqual([AUTOSTART, 'Sprite']);
    for (const row of boxes) {
      expect(row.control.right).toBeCloseTo(row.innerRight, 0);
      expect(row.control.left).toBeGreaterThan(row.label.right);
    }
    // The row's text is the label: clicking it toggles the box.
    const before = await settings.locator('[data-testid="settings-sprite-enabled"]').isChecked();
    await settings.getByText('Sprite', { exact: true }).click();
    expect(await settings.locator('[data-testid="settings-sprite-enabled"]').isChecked()).toBe(!before);
  });

  // "Nothing inside the window repeats the title: the title bar carries it."
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

  // "The window is sized to the form and is not resizable."
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

  // "The banner has its line above the buttons whether or not it has anything
  // to say, so the window never changes size."
  it('keeps the banner line and the window size when a save is refused', async () => {
    const settings = await openSettings();
    // The banner's line is the block right above the button row.
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

  // "... so the window never changes size." A refused hotkey echoes what was
  // typed, and what was typed can be anything: the message has to stay on
  // its line whatever it quotes, the buttons where they were, the form
  // unscrolled. Two shapes of anything: one word that no line can hold,
  // which pushes sideways, and a run of words, which wraps and pushes up.
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

  // "`Cancel`, then `Save`, at the bottom right; `Save` is the one filled
  // button ..."
  it('has Cancel then Save at the bottom right, Save the one filled button', async () => {
    const settings = await openSettings();
    const cancel = await settings.locator('[data-testid="settings-cancel"]').boundingBox();
    const save = await settings.locator('[data-testid="settings-save"]').boundingBox();
    const groups = await readGroups(settings);
    const { innerHeight, main } = await settings.evaluate(() => {
      const s = getComputedStyle(document.querySelector('main'));
      return { innerHeight: window.innerHeight, main: { bottom: parseFloat(s.paddingBottom) } };
    });
    // Cancel, then Save, on one line.
    expect(cancel.x + cancel.width).toBeLessThanOrEqual(save.x);
    expect(Math.abs(cancel.y - save.y)).toBeLessThan(1);
    // At the right: flush with the form's right edge; at the bottom: nothing
    // below but the margin.
    expect(save.x + save.width).toBeCloseTo(groups[2].box.right, 0);
    expect(save.y + save.height).toBeCloseTo(innerHeight - main.bottom, 0);
    // The one filled button: a fill of its own, unlike Cancel's.
    const fills = await settings.evaluate(() => {
      const bg = (id) => getComputedStyle(document.querySelector(`[data-testid="${id}"]`)).backgroundColor;
      return { save: bg('settings-save'), cancel: bg('settings-cancel') };
    });
    expect(fills.save).not.toBe('rgba(0, 0, 0, 0)');
    expect(fills.save).not.toBe(fills.cancel);
  });
});
