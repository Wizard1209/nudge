import { PNG } from "pngjs"
import type { Page } from "puppeteer"
import { CARD_REGION } from "./judge"

/** Decode PNG buffer into raw pixels. */
export function decode(buf: Buffer): PNG {
    return PNG.sync.read(buf)
}

/** Average brightness (0..255) of an (x, y, w, h) patch on the decoded PNG. */
export function avgBrightness(png: PNG, x: number, y: number, w: number, h: number): number {
    let sum = 0
    let count = 0
    for (let dy = 0; dy < h; dy++) {
        for (let dx = 0; dx < w; dx++) {
            const i = ((y + dy) * png.width + (x + dx)) * 4
            sum += (png.data[i] + png.data[i + 1] + png.data[i + 2]) / 3
            count++
        }
    }
    return sum / count
}

/**
 * Peak brightness (0..255) within a patch — useful for detecting antialiased
 * glyph strokes whose average is dominated by dark background pixels.
 */
export function peakBrightness(png: PNG, x: number, y: number, w: number, h: number): number {
    let peak = 0
    for (let dy = 0; dy < h; dy++) {
        for (let dx = 0; dx < w; dx++) {
            const i = ((y + dy) * png.width + (x + dx)) * 4
            const b = (png.data[i] + png.data[i + 1] + png.data[i + 2]) / 3
            if (b > peak) peak = b
        }
    }
    return peak
}

// ─── Card visibility (replaces PROMPT_CARD_PRESENT / PROMPT_CARD_ABSENT) ───
//
// Measured on the current wallpaper + dark card fill (deterministic seed):
//   - card VISIBLE: avgBrightness(CARD_REGION) ≈ 42
//   - card HIDDEN : avgBrightness(CARD_REGION) ≈ 60
//
// Threshold 50 sits in the middle with ~8 units of margin on each side.
// If the wallpaper changes, re-run probe-card-brightness.ts to recalibrate.
const CARD_HIDDEN_BRIGHTNESS_MIN = 50

async function cardRegionBrightness(page: Page): Promise<number> {
    const buf = (await page.screenshot()) as Buffer
    return avgBrightness(decode(buf), CARD_REGION.x, CARD_REGION.y, CARD_REGION.w, CARD_REGION.h)
}

export async function assertCardHidden(page: Page): Promise<void> {
    const b = await cardRegionBrightness(page)
    console.log(`assertCardHidden: brightness=${b.toFixed(1)} (expect > ${CARD_HIDDEN_BRIGHTNESS_MIN})`)
    if (b <= CARD_HIDDEN_BRIGHTNESS_MIN) {
        throw new Error(`card still visible: CARD_REGION brightness=${b.toFixed(1)} (expected > ${CARD_HIDDEN_BRIGHTNESS_MIN})`)
    }
}

export async function assertCardVisible(page: Page): Promise<void> {
    const b = await cardRegionBrightness(page)
    console.log(`assertCardVisible: brightness=${b.toFixed(1)} (expect < ${CARD_HIDDEN_BRIGHTNESS_MIN})`)
    if (b >= CARD_HIDDEN_BRIGHTNESS_MIN) {
        throw new Error(`card hidden: CARD_REGION brightness=${b.toFixed(1)} (expected < ${CARD_HIDDEN_BRIGHTNESS_MIN})`)
    }
}
