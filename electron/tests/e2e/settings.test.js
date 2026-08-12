import { describe, it, expect, beforeAll, afterAll, beforeEach } from 'vitest';
import { launchBrowser, createPage } from './fixtures/browser.js';

const STORAGE_KEY = 'nudge-config';

// §5 has one canonical spelling per platform, and the browser under test runs
// on this host — so the expected string is the host's spelling. Asserting one
// of them everywhere would only prove the test knows what a Mac looks like.
const MAC = process.platform === 'darwin';
const spell = (mac, win) => (MAC ? mac : win);

/** Read the persisted config out of the page's localStorage. */
async function readStored(page) {
    const raw = await page.evaluate((k) => localStorage.getItem(k), STORAGE_KEY);
    return raw ? JSON.parse(raw) : null;
}

/** Write a config into localStorage without touching the form — this is the
 * "saved value" every cancelling path is supposed to re-read. */
async function writeStored(page, config) {
    await page.evaluate(
        (k, cfg) => localStorage.setItem(k, JSON.stringify(cfg)),
        STORAGE_KEY,
        config,
    );
}

const fieldValue = (page) =>
    page.$eval('[data-testid="settings-hotkey"]', (el) => el.value);
const recorderState = (page) =>
    page.$eval('[data-testid="settings-hotkey-record"]', (el) => el.dataset.recorder);
const bannerText = (page) =>
    page.$eval('[data-testid="settings-banner"]', (el) => el.textContent).catch(() => null);

async function arm(page) {
    await page.click('[data-testid="settings-hotkey-record"]');
    expect(await recorderState(page)).toBe('armed');
}

/** Press one physical key with modifiers held — a press edge, the only
 * keyboard input an armed recorder accepts. */
async function pressChord(page, modifiers, code) {
    for (const m of modifiers) await page.keyboard.down(m);
    await page.keyboard.press(code);
    for (const m of [...modifiers].reverse()) await page.keyboard.up(m);
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
        await page.bringToFront();
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

    it('prints the accepted vocabulary under the field (§9)', async () => {
        const text = await page.$eval(
            '[data-testid="settings-hotkey-vocabulary"]',
            (el) => el.textContent,
        );
        expect(text).toContain(spell('⌃ ⌥ ⇧ ⌘', 'Ctrl, Alt, Shift, Win'));
        expect(text).toContain('← ↑ → ↓');
        expect(text).toContain("- = [ ] \\ ; ' , . / `");
    });

    it('seeds the default config on Save with no edits', async () => {
        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored).toEqual({
            hotkey: spell('⌃⇧Space', 'Ctrl+Shift+Space'),
            default_interval_minutes: 10,
            autostart: false,
        });
    });

    it('re-spells a stored hotkey for this platform on load (§9)', async () => {
        await writeStored(page, {
            hotkey: 'Alt+Cmd+J',
            default_interval_minutes: 10,
            autostart: false,
        });
        await page.reload({ waitUntil: 'networkidle0' });
        expect(await fieldValue(page)).toBe(spell('⌥⌘J', 'Alt+Win+J'));
    });

    it('persists an interval edit on Save', async () => {
        await page.click('[data-testid="settings-interval"]', { clickCount: 3 });
        await page.type('[data-testid="settings-interval"]', '15');
        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored.default_interval_minutes).toBe(15);
    });

    it('refuses to save a hotkey the app cannot register, and says the vocabulary', async () => {
        await page.click('[data-testid="settings-hotkey"]', { clickCount: 3 });
        await page.type('[data-testid="settings-hotkey"]', 'Ctrl+Numpad5');
        await page.click('[data-testid="settings-save"]');
        expect(await bannerText(page)).toContain('Numpad5');
        expect(await readStored(page)).toBeNull();
    });

    describe('the recorder', () => {
        it('records a chord from real key presses and persists it on Save', async () => {
            await arm(page);
            await pressChord(page, ['Control', 'Shift'], 'KeyA');

            expect(await fieldValue(page)).toBe(spell('⌃⇧A', 'Ctrl+Shift+A'));
            expect(await recorderState(page)).toBe('idle');

            await page.click('[data-testid="settings-save"]');
            expect((await readStored(page)).hotkey).toBe(spell('⌃⇧A', 'Ctrl+Shift+A'));
        });

        it('records the task doc examples: ⌥⌘J, ⌥⌘← and ⇧/', async () => {
            const cases = [
                [['Alt', 'Meta'], 'KeyJ', spell('⌥⌘J', 'Alt+Win+J')],
                [['Alt', 'Meta'], 'ArrowLeft', spell('⌥⌘←', 'Alt+Win+Left')],
                [['Shift'], 'Slash', spell('⇧/', 'Shift+/')],
            ];
            for (const [mods, code, expected] of cases) {
                await arm(page);
                await pressChord(page, mods, code);
                expect(await fieldValue(page), code).toBe(expected);
            }
        });

        it('follows the physical key, not the character it types', async () => {
            // Puppeteer's `KeyJ` is a cap, not a letter: `text` is what a
            // layout would produce, and the recorder must ignore it the way it
            // ignores the `о` a Russian layout puts on this cap.
            await arm(page);
            await page.keyboard.down('Alt');
            await page.keyboard.down('Meta');
            await page.keyboard.press('KeyJ', { text: 'о' });
            await page.keyboard.up('Meta');
            await page.keyboard.up('Alt');
            expect(await fieldValue(page)).toBe(spell('⌥⌘J', 'Alt+Win+J'));
        });

        it('takes a bare F13 and refuses a bare letter by name, staying armed', async () => {
            await arm(page);
            await page.keyboard.press('KeyJ');
            expect(await recorderState(page)).toBe('armed');
            expect(await bannerText(page)).toContain('J needs a modifier');

            // Still armed: the next real chord records.
            await pressChord(page, ['Control'], 'KeyK');
            expect(await fieldValue(page)).toBe(spell('⌃K', 'Ctrl+K'));

            await arm(page);
            await page.keyboard.press('F13');
            expect(await fieldValue(page)).toBe('F13');
            expect(await recorderState(page)).toBe('idle');
        });

        it('refuses the keypad by name and never reads it as the digit row', async () => {
            await arm(page);
            await pressChord(page, ['Alt'], 'Numpad5');
            expect(await bannerText(page)).toContain('Keypad 5');
            expect(await recorderState(page)).toBe('armed');
            expect(await fieldValue(page)).toBe(spell('⌃⇧Space', 'Ctrl+Shift+Space'));

            // The digit row is a different key and does record.
            await pressChord(page, ['Alt'], 'Digit5');
            expect(await fieldValue(page)).toBe(spell('⌥5', 'Alt+5'));
        });

        it('does nothing while only modifiers are held', async () => {
            await arm(page);
            const before = await fieldValue(page);
            await page.keyboard.down('Alt');
            await page.keyboard.down('Meta');
            await page.keyboard.up('Meta');
            await page.keyboard.up('Alt');
            expect(await recorderState(page)).toBe('armed');
            expect(await fieldValue(page)).toBe(before);
            expect(await bannerText(page)).toBeNull();
        });

        it('cancels on bare Escape and puts the saved hotkey back', async () => {
            const before = await fieldValue(page);
            await arm(page);
            await page.keyboard.press('Escape');
            expect(await recorderState(page)).toBe('idle');
            expect(await fieldValue(page)).toBe(before);
        });

        it('re-reads the saved value on every cancelling path, rather than remembering one', async () => {
            // The config changes on "disk" while the recorder is armed. What
            // comes back is that value, not the one the field held at arming.
            for (const cancelIt of [
                async () => page.keyboard.press('Escape'),
                async () => page.click('[data-testid="settings-hotkey-record"]'),
                // The form's own Cancel is a cancelling path too: it must put
                // back what is saved *now*, not the value the page loaded.
                async () => page.click('[data-testid="settings-cancel"]'),
            ]) {
                await arm(page);
                await writeStored(page, {
                    hotkey: 'Alt+F5',
                    default_interval_minutes: 10,
                    autostart: false,
                });
                await cancelIt();
                expect(await recorderState(page)).toBe('idle');
                expect(await fieldValue(page)).toBe(spell('⌥F5', 'Alt+F5'));
                await page.evaluate(() => localStorage.clear());
                await page.reload({ waitUntil: 'networkidle0' });
            }
        });

        it('cancels when the window loses focus', async () => {
            await arm(page);
            // Headless tabs do not take focus from each other (bringToFront
            // fires no blur), so focus really moves out of the top window the
            // one way it can here: into a subframe. The event the recorder
            // sees is the browser's own, not one the test dispatched.
            await page.evaluate(async () => {
                const frame = document.createElement('iframe');
                frame.srcdoc = '<input id="x">';
                document.body.appendChild(frame);
                await new Promise((resolve) => { frame.onload = resolve; });
                frame.contentWindow.focus();
                frame.contentDocument.getElementById('x').focus();
                await new Promise((resolve) => { setTimeout(resolve, 50); });
            });
            expect(await recorderState(page)).toBe('idle');
            expect(await fieldValue(page)).toBe(spell('⌃⇧Space', 'Ctrl+Shift+Space'));
        });

        it('swallows the keys it records (⌘C records instead of copying)', async () => {
            await arm(page);
            await pressChord(page, ['Meta'], 'KeyC');
            expect(await fieldValue(page)).toBe(spell('⌘C', 'Win+C'));
            // Nothing typed into the field either: it holds the combination,
            // not the letter.
            expect(await fieldValue(page)).not.toContain('c');
        });
    });

    it('toggles autostart immediately, without Save', async () => {
        await page.click('[data-testid="settings-autostart"]');
        const stored = await readStored(page);
        expect(stored.autostart).toBe(true);
    });
});
