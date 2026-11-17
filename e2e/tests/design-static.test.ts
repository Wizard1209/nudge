import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, peakBrightness, assertCardVisible } from "../fixtures/pixels"
import { CARD_REGION } from "../fixtures/judge"
import { wait } from "../fixtures/actions"

describe("static initial-load appearance", () => {
    test("the card is on screen", async ({ nudgeFile }) => {
        await wait(700)
        await assertCardVisible(nudgeFile.page)
    })

    test("three rows, separated by two dividers, the card edge above and below (spec §2)", async ({ nudgeFile }) => {
        await wait(700)
        const png = decode((await nudgeFile.page.screenshot()) as Buffer)
        const x = 600
        const col: number[] = []
        for (let y = CARD_REGION.y; y < CARD_REGION.y + CARD_REGION.h; y++) {
            const i = (y * png.width + x) * 4
            col.push((png.data[i] + png.data[i + 1] + png.data[i + 2]) / 3)
        }
        const cardTop = 150
        const peaks: number[] = []
        for (let k = 2; k < col.length - 2; k++) {
            const y = CARD_REGION.y + k
            if (y <= cardTop + 4) continue
            const here = col[k]
            if (here - col[k - 2] > 6 && here - col[k + 2] > 6) peaks.push(y)
        }
        const lines = peaks.filter((y, i) => i === 0 || y - peaks[i - 1] > 2)
        console.log(`dividers at y=${lines.join(",")}`)
        expect(lines.length).toBe(3)
        const [d1, d2, bottom] = lines
        const row1 = d1 - cardTop
        const row2 = d2 - d1
        const row3 = bottom - d2
        console.log(`row heights: ${row1} ${row2} ${row3}`)
        expect(Math.abs(row1 - row2)).toBeLessThanOrEqual(2)
        expect(Math.abs(row2 - row3)).toBeLessThanOrEqual(2)
    })

    test("placeholder hint text is legible", async ({ nudgeFile }) => {
        await wait(700)
        const png = decode((await nudgeFile.page.screenshot()) as Buffer)
        const peak = peakBrightness(png, 180, 200, 100, 20)
        console.log(`placeholder-hint peak: ${peak.toFixed(1)}`)
        expect(peak).toBeGreaterThan(140)
    })
})
