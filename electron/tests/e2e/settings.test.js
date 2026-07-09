import { describe, it, expect, beforeAll, afterAll, beforeEach } from 'vitest';
import { launchBrowser, createPage } from './fixtures/browser.js';

const STORAGE_KEY = 'nudge-config';

/** Read the persisted config out of the page's localStorage. */
async function readStored(page) {
    const raw = await page.evaluate((k) => localStorage.getItem(k), STORAGE_KEY);
    return raw ? JSON.parse(raw) : null;
}

describe('settings sub-app', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser, '/settings.html'));
    });

    afterAll(async () => {
        await browser?.close();
    });

    // Each test starts from a clean slate: clear storage, reload the sub-app.
    beforeEach(async () => {
        await page.evaluate(() => localStorage.clear());
        await page.reload({ waitUntil: 'networkidle0' });
        await page.waitForSelector('[data-testid="settings-save"]');
    });

    it('renders the settings form controls', async () => {
        expect(await page.$('[data-testid="settings-hotkey"]')).not.toBeNull();
        expect(await page.$('[data-testid="settings-hotkey-record"]')).not.toBeNull();
        expect(await page.$('[data-testid="settings-interval"]')).not.toBeNull();
        expect(await page.$('[data-testid="settings-autostart"]')).not.toBeNull();
        expect(await page.$('[data-testid="settings-save"]')).not.toBeNull();
        expect(await page.$('[data-testid="settings-cancel"]')).not.toBeNull();
    });

    it('seeds the default config on Save with no edits', async () => {
        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored).toEqual({
            hotkey: 'Ctrl+Shift+Space',
            default_interval_minutes: 10,
            autostart: false,
        });
    });

    it('persists an interval edit on Save', async () => {
        await page.click('[data-testid="settings-interval"]', { clickCount: 3 });
        await page.type('[data-testid="settings-interval"]', '15');
        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored.default_interval_minutes).toBe(15);
    });

    it('records a hotkey chord and persists it on Save', async () => {
        await page.click('[data-testid="settings-hotkey-record"]');
        await page.keyboard.down('Control');
        await page.keyboard.down('Shift');
        await page.keyboard.press('KeyA');
        await page.keyboard.up('Shift');
        await page.keyboard.up('Control');

        const shown = await page.$eval(
            '[data-testid="settings-hotkey"]',
            (el) => el.value,
        );
        expect(shown).toBe('Ctrl+Shift+A');

        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored.hotkey).toBe('Ctrl+Shift+A');
    });

    it('cancels recording on bare Escape and restores the prior hotkey', async () => {
        const before = await page.$eval(
            '[data-testid="settings-hotkey"]',
            (el) => el.value,
        );
        await page.click('[data-testid="settings-hotkey-record"]');
        await page.keyboard.press('Escape');
        const after = await page.$eval(
            '[data-testid="settings-hotkey"]',
            (el) => el.value,
        );
        expect(after).toBe(before);
    });

    it('toggles autostart immediately, without Save', async () => {
        await page.click('[data-testid="settings-autostart"]');
        const stored = await readStored(page);
        expect(stored.autostart).toBe(true);
    });
});
