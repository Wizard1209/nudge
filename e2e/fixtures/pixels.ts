import { PNG } from "pngjs"

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
