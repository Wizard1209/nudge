import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, peakBrightness, assertCardVisible } from "../fixtures/pixels"
import { CARD_REGION } from "../fixtures/judge"
import { wait } from "../fixtures/actions"

// LLM-judged asserts on the same fresh-load state (placeholder copy, rounded
// corners) live in design-popup.judge.test.ts; the judge group needs
// OPENAI_API_KEY. Everything measurable is measured here.
describe("static initial-load appearance", () => {
    test("the card is on screen", async ({ nudgeFile }) => {
        await wait(700)
        await assertCardVisible(nudgeFile.page)
    })

    test("three rows, separated by two dividers, the card edge above and below (spec §2)", async ({ nudgeFile }) => {
        await wait(700)
        const png = decode((await nudgeFile.page.screenshot()) as Buffer)
        // Walk down a clean column of the card (right of any text, left of
        // the edge) and collect the rows that are brighter than both of their
        // neighbours by a clear margin — the dividers are 1px hairlines
        // lighter than the fill. The card's top and bottom edges sit outside
        // the scanned band on purpose: only the interior lines are counted.
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
        // Merge runs of adjacent peak rows (antialiasing can spread a line).
        const lines = peaks.filter((y, i) => i === 0 || y - peaks[i - 1] > 2)
        console.log(`dividers at y=${lines.join(",")}`)
        expect(lines.length).toBe(3) // two dividers + the card's bottom edge
        const [d1, d2, bottom] = lines
        const row1 = d1 - cardTop
        const row2 = d2 - d1
        const row3 = bottom - d2
        console.log(`row heights: ${row1} ${row2} ${row3}`)
        // Spec §2: rows of equal height. Allow 2px for the hairline's own rows.
        expect(Math.abs(row1 - row2)).toBeLessThanOrEqual(2)
        expect(Math.abs(row2 - row3)).toBeLessThanOrEqual(2)
    })

    test("placeholder hint text is legible", async ({ nudgeFile }) => {
        // Hint text appears once egui has settled.
        await wait(700)
        const png = decode((await nudgeFile.page.screenshot()) as Buffer)
        // Row 2 ("Хуйня?" placeholder, spec §3) is unfocused on initial render → hint visible.
        // Card left ≈ 160; glyphs start ~20 px in, so band is x≈180..270, y≈200..220.
        const peak = peakBrightness(png, 180, 200, 100, 20)
        console.log(`placeholder-hint peak: ${peak.toFixed(1)}`)
        // Legible AA glyphs peak ≥140; default dim hint peaks ~100.
        expect(peak).toBeGreaterThan(140)
    })
})
