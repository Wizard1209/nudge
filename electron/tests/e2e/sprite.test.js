import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { launchBrowser, createPage } from './fixtures/browser.js';
import { SPRITE_FIXTURE_PATH, SPRITE_URL, SPRITE_VIEWPORT, pixelAt, isGreen } from './fixtures/sprite.js';

// The sprite is placed relative to the card by `x`/`y` (its top-left from the
// card's top-left). At (-41, -277) sprite row 277 lands on the card's top edge
// (the body ends right above it, the fingers hang over it) and sprite column 41
// on the card's left edge (the hair tail hangs down just outside it). The
// fixture is the real sprite's silhouette in flat #00FF00 with binary alpha,
// so every probe point below is either green or not.
describe('sprite over the card', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser, SPRITE_URL, SPRITE_VIEWPORT));
        await page.waitForSelector('[data-testid="sprite"]');
    });

    afterAll(async () => {
        await browser?.close();
    });

    async function rects() {
        return page.evaluate(() => {
            const r = (sel) => document.querySelector(sel).getBoundingClientRect().toJSON();
            return { sprite: r('[data-testid="sprite"]'), card: r('[data-testid="nudge"]') };
        });
    }

    it('draws the sprite 1:1 at its natural size', async () => {
        const { sprite } = await rects();
        expect([sprite.width, sprite.height]).toEqual([720, 430]);
    });

    it('puts the card top on sprite row 277 and its left edge on column 41', async () => {
        const { sprite, card } = await rects();
        expect(card.top - sprite.top).toBe(277);
        expect(card.left - sprite.left).toBe(41);
    });

    it('makes the card cardWidth wide and keeps it at its spec spot', async () => {
        const { card } = await rects();
        expect(card.width).toBe(679);
        // Spec §1: centred, top edge at 25vh + 80px.
        expect(card.left + card.width / 2).toBe(SPRITE_VIEWPORT.width / 2);
        expect(card.top).toBe(SPRITE_VIEWPORT.height * 0.25 + 80);
    });

    it('keeps the hair tail outside the card', async () => {
        const { sprite, card } = await rects();
        // Rightmost tail pixel below the grip is column 40.
        expect(sprite.left + 40).toBeLessThan(card.left);
    });

    it('paints the fingers on top of the card', async () => {
        const { sprite, card } = await rects();
        const [x, y] = [sprite.left + 300, sprite.top + 300];
        expect(x > card.left && y > card.top).toBe(true);
        expect(isGreen(await pixelAt(page, x, y))).toBe(true);
    });

    it('leaves no gap between the body and the card edge', async () => {
        const { sprite } = await rects();
        const x = sprite.left + 500;
        expect(isGreen(await pixelAt(page, x, sprite.top + 276))).toBe(true);
        expect(isGreen(await pixelAt(page, x, sprite.top + 277))).toBe(false);
    });

    it('hides the sprite together with the card on Esc', async () => {
        await page.keyboard.press('Escape');
        expect(await page.$('[data-testid="nudge"]')).toBeNull();
        expect(await page.$('[data-testid="sprite"]')).toBeNull();
    });
});

// Any placement works: here the sprite hangs below-right of a default card.
describe('sprite at another placement', () => {
    let browser;
    let page;

    beforeAll(async () => {
        browser = await launchBrowser();
        ({ page } = await createPage(browser, `/?sprite=${SPRITE_FIXTURE_PATH}&x=200&y=120`, SPRITE_VIEWPORT));
        await page.waitForSelector('[data-testid="sprite"]');
    });

    afterAll(async () => {
        await browser?.close();
    });

    it('puts the sprite top-left at (x, y) from the card top-left, card at default width', async () => {
        const g = await page.evaluate(() => {
            const r = (sel) => document.querySelector(sel).getBoundingClientRect().toJSON();
            return { sprite: r('[data-testid="sprite"]'), card: r('[data-testid="nudge"]') };
        });
        expect([g.sprite.left - g.card.left, g.sprite.top - g.card.top]).toEqual([200, 120]);
        expect(g.card.width).toBe(480);
        expect(g.card.top).toBe(SPRITE_VIEWPORT.height * 0.25 + 80);
    });
});
