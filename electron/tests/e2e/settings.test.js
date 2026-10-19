import { describe, it, expect, beforeAll, afterAll, beforeEach } from 'vitest';
import { launchBrowser, createPage } from './fixtures/browser.js';
import { SPRITE_FIXTURE_PATH } from './fixtures/sprite.js';
import { opaqueRgbaPng, rgbPng } from '../fixtures/png.js';
import { SPRITE_ERROR } from '../../src/shared/sprite.ts';

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
const savedNote = (page) =>
    page.$eval('[data-testid="settings-saved"]', (el) => el.textContent.trim()).catch(() => null);
const saveDisabled = (page) =>
    page.$eval('[data-testid="settings-save"]', (el) => el.disabled);

/** Replace a field's text by typing into it, so the page sees real edits. */
async function retype(page, id, text) {
    await page.click(`[data-testid="${id}"]`, { clickCount: 3 });
    await page.type(`[data-testid="${id}"]`, text);
}

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

    it('writes the whole config on the first Save, defaults included', async () => {
        await retype(page, 'settings-interval', '15');
        await page.click('[data-testid="settings-save"]');
        const stored = await readStored(page);
        expect(stored).toEqual({
            hotkey: spell('⌃⇧Space', 'Ctrl+Shift+Space'),
            default_interval_minutes: 15,
            autostart: false,
            sprite: { enabled: false, path: '', x: 0, y: 0, card_width: 480 },
        });
    });

    describe('saving (§9)', () => {
        // §9 Saving: "`Save` is live only while the form differs from the
        // file, so a form that reads as unedited is one whose content is on
        // disk."
        it('keeps Save greyed while the form matches the file', async () => {
            expect(await saveDisabled(page)).toBe(true);
            await retype(page, 'settings-interval', '15');
            expect(await saveDisabled(page)).toBe(false);
            // Typing the stored value back is no edit: nothing differs.
            await retype(page, 'settings-interval', '10');
            expect(await saveDisabled(page)).toBe(true);
            await retype(page, 'settings-interval', '15');
            await page.click('[data-testid="settings-save"]');
            await page.waitForFunction(() => localStorage.getItem('nudge-config'));
            expect(await saveDisabled(page)).toBe(true);
        });

        // §9 Saving: "A save that reached the file says so — `Saved`, beside
        // the buttons — and the first edit after it takes the note away."
        it('says Saved after a save, until the first edit', async () => {
            expect(await savedNote(page)).toBeNull();
            await retype(page, 'settings-interval', '15');
            await page.click('[data-testid="settings-save"]');
            await page.waitForSelector('[data-testid="settings-saved"]');
            expect(await savedNote(page)).toBe('Saved');
            await page.type('[data-testid="settings-interval"]', '0');
            expect(await savedNote(page)).toBeNull();
        });

        // §9 Saving: "The note is the button's: an autostart click, committed
        // on its own, takes it away like any other edit and raises none."
        it('an autostart click after a save takes the note away and raises none', async () => {
            await retype(page, 'settings-interval', '15');
            await page.click('[data-testid="settings-save"]');
            await page.waitForSelector('[data-testid="settings-saved"]');
            await page.click('[data-testid="settings-autostart"]');
            expect(await savedNote(page)).toBeNull();
            expect((await readStored(page)).autostart).toBe(true);
            expect(await saveDisabled(page)).toBe(true);
        });

        it('does not say Saved for a save that was refused', async () => {
            await retype(page, 'settings-hotkey', 'Ctrl+Numpad5');
            await page.click('[data-testid="settings-save"]');
            expect(await bannerText(page)).toContain('Numpad5');
            expect(await savedNote(page)).toBeNull();
        });

        // §9 Saving: "Return anywhere in the form is `Save`: the same checks,
        // the same write, the same messages as the button."
        it('saves on Return in any field', async () => {
            await retype(page, 'settings-interval', '15');
            await page.keyboard.press('Enter');
            await page.waitForFunction(() => localStorage.getItem('nudge-config'));
            expect((await readStored(page)).default_interval_minutes).toBe(15);
            expect(await savedNote(page)).toBe('Saved');

            await retype(page, 'settings-sprite-x', '7');
            await page.keyboard.press('Enter');
            await page.waitForFunction(() => JSON.parse(localStorage.getItem('nudge-config')).sprite.x === 7);
            expect((await readStored(page)).sprite.x).toBe(7);
        });

        it('runs the same checks on Return, and shows the same message', async () => {
            await retype(page, 'settings-hotkey', 'Ctrl+Numpad5');
            await page.keyboard.press('Enter');
            expect(await bannerText(page)).toContain('Numpad5');
            expect(await readStored(page)).toBeNull();
        });

        it('does nothing on Return while there is nothing to save', async () => {
            await page.focus('[data-testid="settings-interval"]');
            await page.keyboard.press('Enter');
            expect(await readStored(page)).toBeNull();
            expect(await savedNote(page)).toBeNull();
        });

        // §9 Saving: "A focused button or checkbox keeps its own Return —
        // `Cancel` stays `Cancel`."
        it('leaves Return to a focused button or checkbox', async () => {
            await retype(page, 'settings-interval', '15');
            await page.focus('[data-testid="settings-autostart"]');
            await page.keyboard.press('Enter');
            expect(await readStored(page)).toBeNull();
            expect(await savedNote(page)).toBeNull();
            expect(await saveDisabled(page)).toBe(false);

            await page.focus('[data-testid="settings-cancel"]');
            await page.keyboard.press('Enter');
            expect(await readStored(page)).toBeNull();
            expect(await page.$eval('[data-testid="settings-interval"]', (el) => el.value)).toBe('10');
        });

        // §9 Saving: "While the recorder is armed the key is the recorder's —
        // a press, not a save."
        it('leaves Return to an armed recorder', async () => {
            await retype(page, 'settings-interval', '15');
            await arm(page);
            await page.keyboard.press('Enter');
            expect(await recorderState(page)).toBe('armed');
            expect(await bannerText(page)).toContain('Enter needs a modifier');
            expect(await readStored(page)).toBeNull();
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

    describe('sprite section', () => {
        const IDS = [
            'settings-sprite-enabled',
            'settings-sprite-path',
            'settings-sprite-x',
            'settings-sprite-y',
            'settings-card-width',
        ];
        const dataUrl = (buf) => `data:image/png;base64,${buf.toString('base64')}`;

        // Svelte's bind:value listens for `input`; set long values in one go.
        async function setField(id, value) {
            await page.$eval(`[data-testid="${id}"]`, (el, v) => {
                el.value = v;
                el.dispatchEvent(new Event('input', { bubbles: true }));
            }, value);
        }

        async function spriteError() {
            const el = await page.$('[data-testid="settings-sprite-error"]');
            return el ? el.evaluate((e) => e.textContent.trim()) : null;
        }

        it('shows enabled, path, X, Y, card width — in that order, after autostart', async () => {
            const order = await page.evaluate((ids) => {
                const els = ['settings-autostart', ...ids].map((id) => document.querySelector(`[data-testid="${id}"]`));
                if (els.some((e) => !e)) return null;
                return els.slice(1).every((el, i) =>
                    els[i].compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING);
            }, IDS);
            expect(order).toBe(true);
        });

        it('starts off, with an empty path, at (0, 0) and a 480px card', async () => {
            const values = await page.evaluate((ids) => ids.map((id) => {
                const el = document.querySelector(`[data-testid="${id}"]`);
                return el.type === 'checkbox' ? el.checked : el.value;
            }), IDS);
            expect(values).toEqual([false, '', '0', '0', '480']);
        });

        // §10: "`x`, `y` … in points", "`card_width` … in points".
        it('labels X, Y and card width in points, and says so when refusing one', async () => {
            const labels = await page.evaluate((ids) => ids.map((id) =>
                document.querySelector(`[data-testid="${id}"]`).closest('label').querySelector('span').textContent.trim()),
            ['settings-sprite-x', 'settings-sprite-y', 'settings-card-width']);
            expect(labels).toEqual(['X (pt)', 'Y (pt)', 'Card width (pt)']);

            for (const [id, pattern] of [
                ['settings-sprite-x', /X must be a whole number of points/],
                ['settings-sprite-y', /Y must be a whole number of points/],
                ['settings-card-width', /Card width must be a positive whole number of points/],
            ]) {
                await page.reload({ waitUntil: 'networkidle0' });
                await setField(id, '1.5');
                await page.click('[data-testid="settings-save"]');
                await page.waitForSelector('[data-testid="settings-sprite-error"]');
                expect(await spriteError()).toMatch(pattern);
            }
        });

        it('has no Browse button in the browser sub-app', async () => {
            expect(await page.$('[data-testid="settings-sprite-browse"]')).toBeNull();
        });

        it('saves a valid sprite', async () => {
            await page.click('[data-testid="settings-sprite-enabled"]');
            await setField('settings-sprite-path', SPRITE_FIXTURE_PATH);
            await setField('settings-sprite-x', '-41');
            await setField('settings-sprite-y', '-277');
            await setField('settings-card-width', '679');
            await page.click('[data-testid="settings-save"]');
            await page.waitForFunction(() => localStorage.getItem('nudge-config'));
            expect((await readStored(page)).sprite).toEqual({
                enabled: true, path: SPRITE_FIXTURE_PATH, x: -41, y: -277, card_width: 679,
            });
            expect(await spriteError()).toBeNull();
        });

        for (const [name, path, error] of [
            ['an empty path', '', SPRITE_ERROR.noPath],
            ['a non-PNG', '/index.html', SPRITE_ERROR.notPng],
            ['a PNG without alpha', dataUrl(rgbPng()), SPRITE_ERROR.noAlpha],
            ['a fully opaque PNG', dataUrl(opaqueRgbaPng()), SPRITE_ERROR.opaque],
        ]) {
            it(`blocks Save for an enabled sprite with ${name}, showing why`, async () => {
                await page.click('[data-testid="settings-sprite-enabled"]');
                await setField('settings-sprite-path', path);
                await page.click('[data-testid="settings-save"]');
                await page.waitForSelector('[data-testid="settings-sprite-error"]');
                expect(await spriteError()).toBe(error);
                expect(await readStored(page)).toBeNull();
            });
        }

        it('saves a disabled sprite whatever its path', async () => {
            await setField('settings-sprite-path', '/nope.png');
            await page.click('[data-testid="settings-save"]');
            await page.waitForFunction(() => localStorage.getItem('nudge-config'));
            expect((await readStored(page)).sprite).toMatchObject({ enabled: false, path: '/nope.png' });
            expect(await spriteError()).toBeNull();
        });

        it('blocks Save on a non-integer X, inline', async () => {
            await setField('settings-sprite-x', '1.5');
            await page.click('[data-testid="settings-save"]');
            await page.waitForSelector('[data-testid="settings-sprite-error"]');
            expect(await spriteError()).toMatch(/X/);
            expect(await readStored(page)).toBeNull();
        });

        it('clears the error once a fixed form saves', async () => {
            await page.click('[data-testid="settings-sprite-enabled"]');
            await page.click('[data-testid="settings-save"]');
            await page.waitForSelector('[data-testid="settings-sprite-error"]');
            await setField('settings-sprite-path', SPRITE_FIXTURE_PATH);
            await page.click('[data-testid="settings-save"]');
            await page.waitForFunction(() => localStorage.getItem('nudge-config'));
            expect(await spriteError()).toBeNull();
        });
    });
});
