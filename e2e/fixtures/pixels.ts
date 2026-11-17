import { PNG } from "pngjs"
import type { Page } from "puppeteer"
import { CARD_REGION } from "./judge"

export function decode(buf: Buffer): PNG {
    return PNG.sync.read(buf)
}

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
