import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, peakBrightness } from "../fixtures/pixels"
import { wait } from "../fixtures/actions"

// LLM-judged asserts on the same fresh-load state live in
// design-static.judge.test.ts (the judge group needs OPENAI_API_KEY).
describe("static initial-load appearance", () => {
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
