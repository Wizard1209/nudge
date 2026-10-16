// Green silhouette of the real sprite (720×430, #00FF00, alpha 0/255).
export const SPRITE_FIXTURE_PATH = '/tests/e2e/fixtures/sprite-green.png';

// The popup URL for the fixture placed like the real sprite: its top-left at
// (-41, -277) from the card's, on a 679px card.
export const SPRITE_URL = `/?sprite=${SPRITE_FIXTURE_PATH}&x=-41&y=-277&cardWidth=679`;

// Tall enough that the sprite above the card (card top at 25vh, 277px of
// sprite above it) stays inside the viewport.
export const SPRITE_VIEWPORT = { width: 1000, height: 1200 };

/** RGB of one screen pixel, read from a 1×1 screenshot decoded in the page. */
export async function pixelAt(page, x, y) {
    const png = await page.screenshot({ clip: { x, y, width: 1, height: 1 }, encoding: 'base64' });
    return page.evaluate(async (b64) => {
        const img = new Image();
        img.src = `data:image/png;base64,${b64}`;
        await img.decode();
        const ctx = new OffscreenCanvas(1, 1).getContext('2d');
        ctx.drawImage(img, 0, 0);
        const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
        return [r, g, b];
    }, png);
}

export function isGreen([r, g, b]) {
    return g > 200 && r < 60 && b < 60;
}
