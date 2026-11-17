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
        await page.click('[data-testid="field-doing"]');
        await page.type('[data-testid="field-doing"]', 'writing tests');
        await page.type('[data-testid="field-bullshit"]', 'no');

        await page.keyboard.press('Enter');

        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);
        const mins = await page.$eval('[data-testid="field-minutes"]', (el) => el.value);

        expect(doing).toBe('');
        expect(bs).toBe('');
        expect(mins).toBe('10');
    });

    it('Esc hides form and preserves doing/bullshit on reappear', async () => {
        await page.click('[data-testid="field-doing"]');
        await page.type('[data-testid="field-doing"]', 'browsing reddit');
        await page.type('[data-testid="field-bullshit"]', 'yes');

        await page.keyboard.press('Escape');

        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);

        expect(doing).toBe('browsing reddit');
        expect(bs).toBe('yes');
    });

    it('emulated tray appears with countdown when form is hidden', async () => {
        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Escape');

        const tray = await page.waitForSelector('[data-testid="tray"]');
        expect(tray).not.toBeNull();

        const text = await page.$eval('[data-testid="tray"]', (el) => el.textContent.trim());
        expect(text).toMatch(/^\d+:\d{2}$/);
    });

    it('clicking tray shows form with focus on first field', async () => {
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        await new Promise((r) => setTimeout(r, 100));

        const focused = await page.evaluate(() =>
            document.activeElement?.getAttribute('data-testid'),
        );
        expect(focused).toBe('field-doing');
    });

    it('form reappears automatically after timer expires', async () => {
        const minutesField = await page.$('[data-testid="field-minutes"]');
        await minutesField.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05');

        await page.keyboard.press('Enter');

        expect(await page.$('[data-testid="nudge"]')).toBeNull();

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
        expect(await page.$('[data-testid="nudge"]')).not.toBeNull();

        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));

        expect(await page.$('[data-testid="nudge"]')).toBeNull();
        expect(await page.$('[data-testid="tray"]')).not.toBeNull();
    });

    it('Switch preserves doing/bullshit; next open shows the same values', async () => {
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');
        await new Promise((r) => setTimeout(r, 50));

        await page.type('[data-testid="field-doing"]', 'thinking');
        await page.keyboard.press('Tab');
        await page.type('[data-testid="field-bullshit"]', 'maybe');

        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doing = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        const bs = await page.$eval('[data-testid="field-bullshit"]', (el) => el.value);
        expect(doing).toBe('thinking');
        expect(bs).toBe('maybe');
    });

    it('Switch on manually-opened popup leaves original timer running (§4)', async () => {
        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05');

        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await new Promise((r) => setTimeout(r, 100));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();

        await new Promise((r) => setTimeout(r, 4000));
        expect(await page.$('[data-testid="nudge"]')).toBeNull();
    });

    it('Enter (after Switch-preserved values) clears fields on next open', async () => {
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');

        const doingBefore = await page.$eval('[data-testid="field-doing"]', (el) => el.value);
        expect(doingBefore).toBe('thinking');

        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '10');

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
        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-testid="tray"]');

        await new Promise((r) => setTimeout(r, 2500));

        const readCountdown = () =>
            page.$eval('[data-testid="tray"]', (el) => el.textContent.trim());

        const before = await readCountdown();
        await page.click('[data-testid="tray"]');
        await page.waitForSelector('[data-testid="nudge"]');
        await page.click('[data-testid="field-doing"]');

        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-testid="tray"]');

        await new Promise((r) => setTimeout(r, 1200));
        const after = await readCountdown();

        const toSec = (mmss) => {
            const [m, s] = mmss.split(':').map(Number);
            return m * 60 + s;
        };
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

    it('card is 480px wide with corners rounded at 16 (§2)', async () => {
        const { width, radius } = await page.$eval('[data-testid="nudge"]', (el) => {
            const s = getComputedStyle(el);
            return { width: el.getBoundingClientRect().width, radius: s.borderTopLeftRadius };
        });
        expect(width).toBe(480);
        expect(radius).toBe('16px');
    });

    it('card fill is (24,24,27) at a flat 95% where nothing frosts the window (§2)', async () => {
        const bg = await page.$eval('[data-testid="nudge"]', (el) => getComputedStyle(el).backgroundColor);
        expect(bg).toBe('rgba(24, 24, 27, 0.95)');
    });

    it('card contour is a 1px white hairline at 60/255 inside the edge (§2)', async () => {
        const shadow = await page.$eval('[data-testid="nudge"]', (el) => getComputedStyle(el).boxShadow);
        expect(shadow).toBe(
            'rgba(255, 255, 255, 0.235) 0px 0px 0px 1px inset, rgba(0, 0, 0, 0.627) 0px 12px 40px 0px',
        );
    });

    it('rows are 42.5 tall, separated by 1px white lines at 50/255 (§2)', async () => {
        const rows = await page.$$eval('[data-testid="nudge"] input', (els) =>
            els.map((el) => {
                const s = getComputedStyle(el);
                const row = getComputedStyle(el.parentElement);
                return {
                    height: el.getBoundingClientRect().height,
                    font: s.fontSize,
                    lineHeight: s.lineHeight,
                    padding: [s.paddingTop, s.paddingRight, s.paddingBottom, s.paddingLeft],
                    separator: row.borderBottomWidth,
                    separatorColor: row.borderBottomColor,
                };
            }),
        );
        expect(rows).toHaveLength(3);
        for (const r of rows) {
            expect(Math.abs(r.height - 42.5)).toBeLessThanOrEqual(0.5);
            expect(r.font).toBe('16px');
            expect(r.lineHeight).toBe('18.5px');
            expect(r.padding).toEqual(['12px', '20px', '12px', '20px']);
        }
        expect(rows.map((r) => r.separator)).toEqual(['1px', '1px', '0px']);
        expect(rows[0].separatorColor).toBe('rgba(255, 255, 255, 0.196)');
        expect(rows[1].separatorColor).toBe(rows[0].separatorColor);
        const cardHeight = await page.$eval('[data-testid="nudge"]', (el) => el.getBoundingClientRect().height);
        expect(Math.abs(cardHeight - 129.5)).toBeLessThanOrEqual(0.5);
    });

    it('placeholders are gray 170, typed text is (235,235,240) (§3)', async () => {
        await page.type('[data-testid="field-doing"]', 'x');
        const { placeholder, text } = await page.$eval('[data-testid="field-bullshit"]', (el) => ({
            placeholder: getComputedStyle(el, '::placeholder').color,
            text: getComputedStyle(el).color,
        }));
        expect(placeholder).toBe('rgb(170, 170, 170)');
        expect(text).toBe('rgb(235, 235, 240)');
        await page.$eval('[data-testid="field-doing"]', (el) => {
            el.value = '';
            el.dispatchEvent(new Event('input', { bubbles: true }));
        });
    });

    it('card top edge sits at 25% of the viewport + 80px (§1)', async () => {
        const { topY, viewportH } = await page.evaluate(() => {
            const el = document.querySelector('[data-testid="nudge"]');
            const r = el.getBoundingClientRect();
            return { topY: r.top, viewportH: window.innerHeight };
        });
        expect(Math.abs(topY - (viewportH * 0.25 + 80))).toBeLessThanOrEqual(1);
    });

    it('card draws no blur of its own: the frost is the compositor\'s (§2)', async () => {
        const filter = await page.$eval(
            '[data-testid="nudge"]',
            (el) => getComputedStyle(el).backdropFilter,
        );
        expect(filter).toBe('none');
    });

    it('card text is set in Ubuntu Light 16px, the egui default face', async () => {
        const { face, checked, family, size, weight } = await page.evaluate(async () => {
            await document.fonts.ready;
            const face = [...document.fonts].find(
                (f) => f.family.replace(/"/g, '') === 'Ubuntu',
            );
            const s = getComputedStyle(document.querySelector('[data-testid="nudge"] input'));
            return {
                face: face ? { status: face.status, weight: face.weight } : null,
                checked: document.fonts.check('300 16px Ubuntu'),
                family: s.fontFamily,
                size: s.fontSize,
                weight: s.fontWeight,
            };
        });
        expect(face).toEqual({ status: 'loaded', weight: '300' });
        expect(checked).toBe(true);
        expect(family).toMatch(/^"?Ubuntu"?/);
        expect(size).toBe('16px');
        expect(weight).toBe('300');
    });

    it('focused row has a visible background tint distinct from unfocused rows', async () => {
        const bgs = await page.$$eval('[data-testid="nudge"] input', (els) =>
            els.map((el) => ({
                testid: el.getAttribute('data-testid'),
                bg: getComputedStyle(el.parentElement, '::before').backgroundColor,
            }))
        );
        const focused = bgs.find((b) => b.testid === 'field-doing');
        const others = bgs.filter((b) => b.testid !== 'field-doing');
        expect(focused.bg).not.toBe(others[0].bg);
        expect(others[0].bg).toBe(others[1].bg);
        const m = focused.bg.match(/(?:\/|,)\s*([\d.]+)\)?\s*$/);
        expect(m, `focused bg = ${focused.bg}`).not.toBeNull();
        expect(parseFloat(m[1])).toBeGreaterThan(0);
    });
});

describe('browser mode opening popup: the timer survives any first close (§4)', () => {
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

        const mins = await page.$('[data-testid="field-minutes"]');
        await mins.click({ clickCount: 3 });
        await page.type('[data-testid="field-minutes"]', '0.05');

        await page.click('[data-testid="field-doing"]');
        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-testid="nudge"]', { hidden: true });

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

        await page.evaluate(() => window.dispatchEvent(new Event('blur')));
        await page.waitForSelector('[data-testid="nudge"]', { hidden: true });

        await page.waitForSelector('[data-testid="nudge"]', { timeout: 6000 });
        expect(await page.$('[data-testid="nudge"]')).not.toBeNull();
        await page.close();
    });
});
