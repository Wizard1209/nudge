import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { writeFileSync } from 'node:fs';
import { launchBrowser, createPage } from './fixtures/browser.js';
import { visualAssert } from './fixtures/judge.js';

// §9 "Window chrome and layout", one prompt per bullet, each quoting the
// sentence it judges and asking only whether the capture shows that. The
// capture is the browser build of settings.html at the window's own size.
// Two negative arms prove the judge can tell the layout apart from a broken
// one: the same prompt must FAIL once the footnote is hidden or the buttons
// are swapped.
const VIEWPORT = { width: 560, height: 582 };

const THREE_GROUPS =
    'Spec: "The form is three groups of rows in rounded boxes, a hairline between rows: the hotkey; the interval and autostart; the sprite (checkbox, PNG, position)." '
    + 'Assertion: the form is exactly three rounded, bordered boxes stacked vertically. The first box holds one row (a hotkey field with a Record button). The second holds two rows (an interval, and a login/launch checkbox). The third holds three rows (a sprite checkbox, a PNG path, a position). Inside a box, adjacent rows are separated by a thin, faint horizontal divider line running the width of the box.';
const LABEL_LEFT_CONTROL_RIGHT =
    'Spec: "In every row the label sits at the left and the control at the right edge." '
    + 'Assertion: in every row of the boxes, the row\'s text label is at the left side of the row, and its control (a text field, a checkbox, a button, or a group of fields) is at the right side of the row, the controls of all rows ending at one common right edge just inside the box\'s right border. In no row is there empty space between the control and the right side of the box.';
const FOOTNOTE =
    'Spec: "The hotkey vocabulary is a footnote under its group." '
    + 'Assertion: directly below the first (topmost) box, outside it and before the second box begins, there is a small line of dim text listing modifier keys and key names (what may be typed as a hotkey).';
const CHECKBOX_RIGHT =
    'Spec: "A checkbox sits at the right of its row; the row\'s text is its label." '
    + 'Assertion: every checkbox is at the right end of its row, with the row\'s text to its left on the same row. No checkbox is at the left of a row.';
const NO_HEADING =
    'Spec: "Nothing inside the window repeats the title: the title bar carries it." '
    + 'Assertion: there is no heading or title text (such as "Settings") inside the page; the first box starts near the top edge of the page with nothing but a margin above it.';
const BUTTONS =
    'Spec: "`Cancel`, then `Save`, at the bottom right; `Save` is the one filled button." '
    + 'Assertion: at the bottom right of the page there are two buttons, "Cancel" to the left of "Save". "Save" is the only button with a solid, saturated colour fill (not grey); every other button on the page is neutral grey.';
const BANNER_ABOVE_BUTTONS =
    'Spec: "The banner has its line above the buttons whether or not it has anything to say." '
    + 'Assertion: an error message in amber/yellow text is shown between the lowest box and the Cancel/Save buttons — above the buttons, below the last box, not anywhere else.';

describe('settings layout (LLM-judged)', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser, '/settings.html', VIEWPORT));
        await page.waitForSelector('[data-testid="settings-save"]');
    });

    afterAll(async () => {
        await browser?.close();
    });

    async function shot() {
        const png = await page.screenshot({ encoding: 'base64' });
        if (process.env.JUDGE_SHOTS_DIR) {
            writeFileSync(`${process.env.JUDGE_SHOTS_DIR}/${Date.now()}.png`, Buffer.from(png, 'base64'));
        }
        return png;
    }

    async function judge(prompt) {
        const result = await visualAssert(await shot(), prompt);
        console.log(prompt.slice(6, 60), '→', result);
        return result.pass;
    }

    async function withStyle(css, body) {
        const handle = await page.addStyleTag({ content: css });
        try {
            await body();
        } finally {
            await handle.evaluate((el) => el.remove());
        }
    }

    it('three groups of rows in rounded boxes', async () => {
        expect(await judge(THREE_GROUPS)).toBe(true);
    });

    it('label at the left, control at the right edge', async () => {
        expect(await judge(LABEL_LEFT_CONTROL_RIGHT)).toBe(true);
    });

    it('the vocabulary is a footnote under its group', async () => {
        expect(await judge(FOOTNOTE)).toBe(true);
    });

    it('negative arm: a hidden footnote is caught', async () => {
        await withStyle('[data-testid="settings-hotkey-vocabulary"] { visibility: hidden }', async () => {
            expect(await judge(FOOTNOTE)).toBe(false);
        });
    });

    it('checkboxes at the right of their rows', async () => {
        expect(await judge(CHECKBOX_RIGHT)).toBe(true);
    });

    it('nothing inside repeats the title', async () => {
        expect(await judge(NO_HEADING)).toBe(true);
    });

    // Photographed with something to save: greyed, the fill is dimmed (§9
    // Saving), and "the one filled button" is a sentence about the live one.
    it('Cancel, then Save, at the bottom right, Save filled', async () => {
        await page.click('[data-testid="settings-interval"]', { clickCount: 3 });
        await page.type('[data-testid="settings-interval"]', '11');
        expect(await judge(BUTTONS)).toBe(true);
    });

    it('negative arm: swapped buttons are caught', async () => {
        // Save sent to the far left of the button line: the one arrangement the
        // sentence rules out.
        await withStyle('[data-testid="settings-save"] { order: -1; margin-right: auto }', async () => {
            expect(await judge(BUTTONS)).toBe(false);
        });
    });

    it('the banner has its line above the buttons', async () => {
        await page.click('[data-testid="settings-interval"]', { clickCount: 3 });
        await page.type('[data-testid="settings-interval"]', 'x');
        await page.click('[data-testid="settings-save"]');
        await page.waitForSelector('[data-testid="settings-banner"]');
        expect(await judge(BANNER_ABOVE_BUTTONS)).toBe(true);
    });
});
