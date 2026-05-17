import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, peakBrightness } from "../fixtures/pixels"
import {
    visualAssert,
    PROMPT_CARD_PRESENT,
    PROMPT_CARD_FROSTED,
    PROMPT_THREE_ROWS,
    CARD_REGION,
} from "../fixtures/judge"
import { wait } from "../fixtures/actions"

// All four asserts target the same fresh-load state and never mutate the
// popup, DOM, or localStorage — safe to share a single browser per file.
describe("static initial-load appearance", () => {
    test("fresh page load shows the input form card", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_CARD_PRESENT)
        console.log("card-present:", result)
        expect(result.pass).toBe(true)
    })

    test("card is a frosted floating surface", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_CARD_FROSTED)
        console.log("card-frosted:", result)
        expect(result.pass).toBe(true)
    })

    test("card shows three equal-height single-line rows", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_THREE_ROWS, { region: CARD_REGION })
        console.log("three-rows:", result)
        expect(result.pass).toBe(true)
    })

    test("placeholder hint text is legible", async ({ nudgeFile }) => {
        // Hint text appears once egui has settled.
        await wait(700)
        const png = decode((await nudgeFile.page.screenshot()) as Buffer)
        // Row 2 ("Хуйня?") is unfocused on initial render → hint visible.
        // Card left ≈ 160; glyphs start ~20 px in, so band is x≈180..270, y≈200..220.
        const peak = peakBrightness(png, 180, 200, 100, 20)
        console.log(`placeholder-hint peak: ${peak.toFixed(1)}`)
        // Legible AA glyphs peak ≥140; default dim hint peaks ~100.
        expect(peak).toBeGreaterThan(140)
    })
})
