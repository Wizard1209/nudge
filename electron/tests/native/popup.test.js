import { describe, it, expect, afterEach } from 'vitest';
import { launchApp, popupState, openSettingsWindow, settingsState } from './fixtures/electron.js';

// Bug 2: "after three Tabs the window closes."
// The popup is a frameless, always-on-top window with exactly three fields.
// Tabbing off the last field (or Shift+Tab off the first) lets focus leave the
// window → the OS fires a `blur` → §4 Switch-on-blur hides it. The fix is a
// focus trap: Tab/Shift+Tab wrap WITHIN the three fields so focus never escapes
// the window and no stray blur is generated.
//
// Note on the signal: we assert focus stays on a field and the form stays
// mounted — NOT BrowserWindow.isVisible(), which is unreliable under WSLg's
// headless compositor (it reports false even for a freshly shown window). The
// trap invariant is what actually prevents the hide, on every platform.
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

// Bug 1 (the real one): "on first open only half is visible." The card is
// positioned `top-[calc(25vh+80px)]` — correct for the browser build where the
// window IS the full screen, but in Electron the window is sized to the card
// (480×130) and the main process already places it at its spec spot. So the
// offset pushes the card down INSIDE its own window: a transparent strip
// appears above it (the blurred-desktop band) and the last field is clipped
// below the fold. The card must fill its window in Electron.
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

    // The window is the card: 480 wide, and the whole number of points that
    // holds three 42.5 rows and two 1px separators (129.5).
    const { contentBounds } = await popupState(app);
    expect([contentBounds.width, contentBounds.height]).toEqual([480, 130]);
    expect(Math.abs(g.card.height - 129.5)).toBeLessThanOrEqual(0.5);
    // Card flush with the top of its card-sized window — no viewport-relative
    // offset leaking in from the browser layout.
    expect(g.card.top).toBeLessThanOrEqual(4);
    // Every field is fully within the window — nothing below the fold.
    for (const f of g.fields) {
      expect(f.bottom).toBeLessThanOrEqual(g.innerH + 1);
    }
  });
});

// Spec §1, "Which monitor: the one with the keyboard focus — the display
// holding the frontmost application's front window, as Spotlight chooses".
// This host has one display, so the chosen display can only be that one; what
// this case pins down is the plumbing — the addon reads a front window, it is
// ours once a regular window of ours is up, its CG bounds agree with
// Electron's screen DIPs (same top-left origin, no flip), and the popup's
// display is the one that window is on. The multi-display cases live in the
// pure displayFor's unit tests (tests/unit/display.test.ts).
//
// The popup itself never is that window: it takes the keyboard without
// activating the app (see macosPanel.test.js), exactly like Spotlight, so the
// front window stays the one the user was working in. Settings is a regular
// window and does activate the app.
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
      // Activation lands asynchronously after show(); give it a moment.
      let r = await read();
      for (const deadline = Date.now() + 5000; Date.now() < deadline; r = await read()) {
        if (r.front && r.front.x === settings.bounds.x && r.front.y === settings.bounds.y) break;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      // The front window is our Settings window: the app is frontmost, and
      // the addon's CG points are Electron's DIPs with no conversion.
      expect(r.front).toEqual(settings.bounds);
      expect(r.chosen).toBe(r.matching);
    },
  );
});

function activeTestId(popup) {
  return popup.evaluate(
    () => document.activeElement?.getAttribute('data-testid') ?? document.activeElement?.tagName,
  );
}

function formMounted(popup) {
  return popup.evaluate(() => !!document.querySelector('[data-testid="nudge"]'));
}
