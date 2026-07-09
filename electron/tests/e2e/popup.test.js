import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { launchBrowser, createPage } from './fixtures/browser.js';

describe('popup', () => {
    let browser;
    let page;
    let consoleErrors;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page, consoleErrors } = await createPage(browser));
    });

    afterAll(async () => {
        await browser?.close();
    });

    it('renders the popup container', async () => {
        const popup = await page.$('[data-testid="nudge"]');
        expect(popup).not.toBeNull();
    });

    it('renders 3 input fields', async () => {
        const fields = await page.$$('[data-testid="nudge"] input');
        expect(fields).toHaveLength(3);
    });

    it('auto-focuses the first field', async () => {
        const focused = await page.evaluate(() =>
            document.activeElement?.getAttribute('data-testid'),
        );
        expect(focused).toBe('field-doing');
    });

    it('Tab cycles through fields in order', async () => {
        // Focus is on field-doing already
        await page.keyboard.press('Tab');
        const second = await page.evaluate(() =>
            document.activeElement?.getAttribute('data-testid'),
        );
        expect(second).toBe('field-bullshit');

        await page.keyboard.press('Tab');
        const third = await page.evaluate(() =>
            document.activeElement?.getAttribute('data-testid'),
        );
        expect(third).toBe('field-minutes');
    });

    it('Enter hides form and fields are clear on reappear', async () => {
        // Type into fields
        await page.click('[data-testid="field-doing"]');
        await page.type('[data-testid="field-doing"]', 'writing tests');
        await page.type('[data-testid="field-bullshit"]', 'no');

        await page.keyboard.press('Enter');

        // Form should be hidden
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        // Bring form back via tray
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);
        const mins = await page.$eval('[data-testid="field-minutes"]', (el) => el.value);

        expect(doing).toBe('');
        expect(bs).toBe('');
        // nextMinutes is preserved
        expect(mins).toBe('10');
    });

    it('Esc hides form and preserves doing/bullshit on reappear', async () => {
        // Spec §4: Esc shares its row with Switch — both keep doing/bullshit
        // so the next open resumes where the user left off. (Easier to press
        // Esc than to break focus by clicking somewhere else.)
        await page.click('[data-testid="field-doing"]');
        await page.type('[data-testid="field-doing"]', 'browsing reddit');
        await page.type('[data-testid="field-bullshit"]', 'yes');

        await page.keyboard.press('Escape');

        // Form should be hidden
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        // Bring form back via tray
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);

        expect(doing).toBe('browsing reddit');
        expect(bs).toBe('yes');

        // End-state contract for this describe-block chain: form is visible
        // (subsequent tests press their own Esc/Enter to hide it). The
        // preserved field contents are harmless for the tests that follow —
        // they only care about visibility / focus / countdown text.
    });

    // --- Lifecycle tests ---

    it('emulated tray appears with countdown when form is hidden', async () => {
        // Hide form first
        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Escape');

        const tray = await page.waitForSelector('[data-testid="tray"]');
        expect(tray).not.toBeNull();

        // Spec §6: pill content is "only time, M:SS format, no prefix"
        const text = await page.$eval('[data-testid="tray"]', (el) => el.textContent.trim());
        expect(text).toMatch(/^\d+:\d{2}$/);
    });

    it('clicking tray shows form with focus on first field', async () => {
        // Tray should be visible (form hidden from previous test)
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        // Wait a tick for focus
        await new Promise((r) => setTimeout(r, 100));

        const focused = await page.evaluate(() =>
            document.activeElement?.getAttribute('data-testid'),
        );
        expect(focused).toBe('field-doing');
    });

    it('form reappears automatically after timer expires', async () => {
        // Set a very short timer (0.05 min = 3 seconds)
        const minutesField = await page.$('[data-testid="field-minutes"]');
        await minutesField.click({ clickCount: 3 }); // select all
        await page.type('[data-testid="field-minutes"]', '0.05');

        await page.keyboard.press('Enter');

        // Form should be hidden
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        // Wait for timer to fire (~3 seconds + buffer)
        await page.waitForSelector('[data-testid="nudge"]', { timeout: 10_000 });

        const popup = await page.$('[data-testid="nudge"]');
        expect(popup).not.toBeNull();
    });

    it('has no console errors', () => {
        expect(consoleErrors).toHaveLength(0);
    });
});

describe('popup switch', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser));
    });

    afterAll(async () => {
        await browser?.close();
    });

    it('window blur hides popup and reveals tray', async () => {
        // Popup starts visible (auto-focused on field-doing)
        expect(await page.$('[data-testid="nudge"]')).not.toBeNull();

        // Switch: simulate window losing focus
        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));

        expect(await page.$('[data-testid="nudge"]')).toBeNull();
        expect(await page.$('[data-testid="tray"]')).not.toBeNull();
    });

    it('Switch preserves doing/bullshit; next open shows the same values', async () => {
        // Reopen via tray (popup was hidden by previous test)
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');
        await new Promise((r) => setTimeout(r, 50)); // let autofocus settle

        await page.type('[data-testid="field-doing"]', 'thinking');
        await page.keyboard.press('Tab');
        await page.type('[data-testid="field-bullshit"]', 'maybe');

        // Switch
        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        // Reopen — values survive
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);
        expect(doing).toBe('thinking');
        expect(bs).toBe('maybe');
    });

    it('Switch on manually-opened popup leaves original timer running (§4)', async () => {
        // Popup is visible from previous test, which reopened it via the tray
        // (trigger_source = "manual"). Per spec §4, manual-source Switch must
        // NOT restart the timer — the live deadline from test 1's blur is
        // still ticking. Even setting minutes here doesn't change that.
        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05');

        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        // If Switch had restarted the timer to 0.05min (3s), the popup would
        // have reopened by now. It must not.
        await new Promise((r) => setTimeout(r, 4000));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();
    });

    it('Enter (after Switch-preserved values) clears fields on next open', async () => {
        // Previous test's Switch left popup hidden — reopen via tray.
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        // Fields preserved from the earlier Switch.
        const doingBefore = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        expect(doingBefore).toBe('thinking');

        // Reset minutes so the next timer doesn't fire mid-test
        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '10');

        // Enter — should clear doing/bullshit per spec §4 table
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-testid="tray"]');
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);
        expect(doing).toBe('');
        expect(bs).toBe('');
    });
});

describe('trigger source determines whether Esc/Switch restart timer (§4)', () => {
    // Spec §4: Esc/Switch leave the timer alone *unless* the popup was opened
    // by the timer itself. Timer-opened means the deadline is already at zero,
    // so closing without setting a new deadline would re-open the popup
    // instantly — hence the structural exception. Manually-opened popups still
    // have a live timer behind them, so Esc/Switch must NOT reset it.

    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser));
    });

    afterAll(async () => {
        await browser?.close();
    });

    it('Esc on a manually-opened popup keeps original timer (countdown keeps falling)', async () => {
        // Initial popup is auto-shown — trigger_source = "timer" per §4. Enter
        // establishes a 10-min timer (long enough that it can't expire mid-test).
        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-testid="tray"]');

        // Let the pill tick down a couple of seconds so a reset would be visible.
        await new Promise((r) => setTimeout(r, 2500));

        const readCountdown = () =>
            page.$eval('[data-testid="tray"]', (el) => el.textContent.trim());

        const before = await readCountdown();
        // Manual open via tray.
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');
        // Focus a field so the form's keydown handler receives Escape.
        await page.click('[data-testid="field-doing"]');

        // Manual-source Esc must NOT restart the timer.
        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-testid="tray"]');

        // Give the 1-second pill interval a moment so its text refreshes.
        await new Promise((r) => setTimeout(r, 1200));
        const after = await readCountdown();

        const toSec = (mmss) => {
            const [m, s] = mmss.split(':').map(Number);
            return m * 60 + s;
        };
        // After Esc, the deadline is unchanged → countdown keeps falling.
        // If Esc had restarted the timer, `after` would be ~10:00 again, i.e.
        // larger than `before`.
        expect(toSec(after)).toBeLessThan(toSec(before));
    });
});

describe('popup visual', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser));
    });

    afterAll(async () => {
        await browser?.close();
    });

    it('card is 480px wide with subtle rounded corners', async () => {
        const { width, radius } = await page.$eval('[data-testid="nudge"]', (el) => {
            const s = getComputedStyle(el);
            return { width: el.getBoundingClientRect().width, radius: s.borderTopLeftRadius };
        });
        expect(width).toBe(480);
        // rounded-lg = 0.5rem = 8px (matches Win11 frameless window radius)
        expect(radius).toBe('8px');
    });

    it('card has translucent dark background (~80% zinc-900)', async () => {
        const bg = await page.$eval('[data-testid="nudge"]', (el) => getComputedStyle(el).backgroundColor);
        // Tailwind v4 emits oklab/oklch. Easiest invariant: alpha > 0.7 and < 0.95.
        // Either format includes the alpha as a trailing '/ <num>'.
        const match = bg.match(/\/\s*([\d.]+)\)?\s*$/);
        expect(match, `bg = ${bg}`).not.toBeNull();
        const alpha = parseFloat(match[1]);
        expect(alpha).toBeGreaterThanOrEqual(0.7);
        expect(alpha).toBeLessThanOrEqual(0.95);
    });

    it('card top edge sits ~25% from top of viewport (§1)', async () => {
        // Spec §1: top edge of card at 25% of screen height. Card grows
        // downward from that point — explicitly not vertically centred.
        const { topY, viewportH } = await page.evaluate(() => {
            const el = document.querySelector('[data-testid="nudge"]');
            const r = el.getBoundingClientRect();
            return { topY: r.top, viewportH: window.innerHeight };
        });
        const ratio = topY / viewportH;
        // Tolerate ±1% for sub-pixel layout; this is a positional spec, not pixel-perfect.
        expect(ratio).toBeGreaterThanOrEqual(0.24);
        expect(ratio).toBeLessThanOrEqual(0.26);
    });

    it('card has backdrop-filter blur (frosted glass, §2)', async () => {
        const filter = await page.$eval(
            '[data-testid="nudge"]',
            (el) => getComputedStyle(el).backdropFilter,
        );
        // Spec §2: surface is translucent with blur so what's behind shows through, blurred.
        expect(filter).toMatch(/blur\(/);
    });

    it('focused row has a visible background tint distinct from unfocused rows', async () => {
        // First field is auto-focused on mount.
        const bgs = await page.$$eval('[data-testid="nudge"] input', (els) =>
            els.map((el) => ({
                testid: el.getAttribute('data-testid'),
                bg: getComputedStyle(el).backgroundColor,
            }))
        );
        const focused = bgs.find((b) => b.testid === 'field-doing');
        const others = bgs.filter((b) => b.testid !== 'field-doing');
        expect(focused.bg).not.toBe(others[0].bg);
        expect(others[0].bg).toBe(others[1].bg);
        // Focused row should have nonzero alpha (i.e. a tint, not transparent)
        const m = focused.bg.match(/\/\s*([\d.]+)\)?\s*$/);
        expect(m, `focused bg = ${focused.bg}`).not.toBeNull();
        expect(parseFloat(m[1])).toBeGreaterThan(0);
    });
});

describe('first-launch rule: the timer survives any first close (§4)', () => {
    // The bug this guards: the first auto-shown popup was opened as "manual",
    // so closing it with Esc/Switch never armed the timer — "the timer dies
    // after first launch". Per §4 the first popup behaves like a timer fire, so
    // every close path must arm the timer. (Electron-only in production, but the
    // shared INITIAL_TRIGGER_SOURCE drives both surfaces; this exercises it.)

    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
    });
    afterAll(async () => {
        await browser?.close();
    });

    it('Esc on the very first popup arms the timer (popup reopens)', async () => {
        ({ page } = await createPage(browser));
        await page.waitForSelector('[data-testid="nudge"]');

        // Short interval so the rearmed timer fires quickly.
        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05'); // 3s

        // Close the first popup with Esc (the path that used to kill the timer).
        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-testid="nudge"]', { hidden: true });

        // If the timer is alive it re-opens the popup within ~3s; a dead timer
        // never would.
        await page.waitForSelector('[data-testid="nudge"]', { timeout: 6000 });
        expect(await page.$('[data-testid="nudge"]')).not.toBeNull();
        await page.close();
    });

    it('Switch on the very first popup arms the timer (popup reopens)', async () => {
        ({ page } = await createPage(browser));
        await page.waitForSelector('[data-testid="nudge"]');

        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05');

        // Switch = window focus loss.
        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await page.waitForSelector('[data-testid="nudge"]', { hidden: true });

        await page.waitForSelector('[data-testid="nudge"]', { timeout: 6000 });
        expect(await page.$('[data-testid="nudge"]')).not.toBeNull();
        await page.close();
    });
});
