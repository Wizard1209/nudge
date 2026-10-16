import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { writeFileSync } from 'node:fs';
import { launchBrowser, createPage } from './fixtures/browser.js';
import { SPRITE_URL, SPRITE_VIEWPORT } from './fixtures/sprite.js';
import { visualAssert } from './fixtures/judge.js';

const FINGERS_OVER_CARD =
    'Several small bright green finger-like shapes hang down over the top edge of the dark rectangular card and are drawn ON TOP of the card, covering part of it.';
// Judged on a crop of the right half of the grip line, away from the fingers
// that would bridge (and hide) a gap.
const BODY_RESTS_ON_CARD =
    'The image has three colours: bright green at the top, a dark gray card (about rgb 75,75,78) at the bottom, and possibly the light gray page background (about rgb 198,198,198). Assertion: the bright green touches the dark gray card directly along a horizontal line, with NO band of the light gray background (rgb ~198) between them.';
const GRIP_CROP = { x: 495, y: 247, width: 75, height: 60 };
const TAIL_OUTSIDE_CARD =
    "A thin bright green strand hangs down vertically to the LEFT of the dark card, entirely outside it: no part of that strand lies over the card's dark surface.";
const NO_GREEN = 'No bright green shape is visible anywhere in the image.';

// Each prompt asserts one atomic fact. The two negative arms prove the judge
// can tell the layouts apart: the same prompt must FAIL once the card is
// nudged out of place.
describe('sprite over the card (LLM-judged)', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser, SPRITE_URL, SPRITE_VIEWPORT));
        await page.waitForSelector('[data-testid="sprite"]');
        // Light flat backdrop (the art's own gray): on the dark default
        // backdrop the dark card's edges — and any gap — are invisible.
        await page.addStyleTag({ content: 'main { background: #c6c6c6 !important } main > div:first-child { display: none }' });
    });

    afterAll(async () => {
        await browser?.close();
    });

    // The sprite plus the card below it, with a margin around both — or
    // `crop`, a rect in sprite pixels.
    async function shot(crop) {
        const clip = await page.evaluate((crop) => {
            const s = document.querySelector('[data-testid="sprite"]')?.getBoundingClientRect();
            const c = document.querySelector('[data-testid="nudge"]')?.getBoundingClientRect();
            if (crop && s) return { x: s.left + crop.x, y: s.top + crop.y, width: crop.width, height: crop.height };
            const r = s ?? c ?? { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
            const left = Math.max(0, Math.min(r.left, c?.left ?? r.left) - 40);
            const top = Math.max(0, r.top - 40);
            const right = Math.max(r.right, c?.right ?? 0) + 40;
            const bottom = Math.max(r.bottom, c?.bottom ?? 0) + 40;
            return { x: left, y: top, width: right - left, height: bottom - top };
        }, crop);
        const png = await page.screenshot({ clip, encoding: 'base64' });
        if (process.env.JUDGE_SHOTS_DIR) {
            writeFileSync(`${process.env.JUDGE_SHOTS_DIR}/${Date.now()}.png`, Buffer.from(png, 'base64'));
        }
        return png;
    }

    async function nudgeCard(dx, dy) {
        await page.evaluate(([x, y]) => {
            const card = document.querySelector('[data-testid="nudge"]');
            card.style.translate = `${x}px ${y}px`;
        }, [dx, dy]);
    }

    async function judge(prompt, crop) {
        const result = await visualAssert(await shot(crop), prompt);
        console.log(prompt.slice(0, 40), '→', result);
        return result.pass;
    }

    it('fingers hang over the card', async () => {
        expect(await judge(FINGERS_OVER_CARD)).toBe(true);
    });

    it('body rests on the card edge', async () => {
        expect(await judge(BODY_RESTS_ON_CARD, GRIP_CROP)).toBe(true);
    });

    it('negative arm: a 16px gap is caught', async () => {
        await nudgeCard(0, 16);
        try {
            expect(await judge(BODY_RESTS_ON_CARD, GRIP_CROP)).toBe(false);
        } finally {
            await nudgeCard(0, 0);
        }
    });

    it('hair tail hangs outside the card', async () => {
        expect(await judge(TAIL_OUTSIDE_CARD)).toBe(true);
    });

    it('negative arm: a card under the tail is caught', async () => {
        await nudgeCard(-41, 0);
        try {
            expect(await judge(TAIL_OUTSIDE_CARD)).toBe(false);
        } finally {
            await nudgeCard(0, 0);
        }
    });

    it('Esc hides the sprite with the card', async () => {
        await page.keyboard.press('Escape');
        expect(await judge(NO_GREEN)).toBe(true);
    });
});
