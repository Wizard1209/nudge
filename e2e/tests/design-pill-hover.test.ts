import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"

// Pill approx at (740, 560, w≈50, h≈26). Sample the centre.
const PILL_X = 748
const PILL_Y = 566
const SAMPLE_SIZE = 16

test("hovering the bottom-right pill makes its background lighter", async ({ nudge }) => {
    // Dismiss the form
    await nudge.page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 600))

    // Move mouse far away (top-left)
    await nudge.page.mouse.move(20, 20)
    await new Promise((r) => setTimeout(r, 400))
    const before = decode((await nudge.page.screenshot()) as Buffer)

    // Hover over the pill
    await nudge.page.mouse.move(PILL_X + SAMPLE_SIZE / 2, PILL_Y + SAMPLE_SIZE / 2)
    await new Promise((r) => setTimeout(r, 600))
    const after = decode((await nudge.page.screenshot()) as Buffer)

    const b0 = avgBrightness(before, PILL_X, PILL_Y, SAMPLE_SIZE, SAMPLE_SIZE)
    const b1 = avgBrightness(after, PILL_X, PILL_Y, SAMPLE_SIZE, SAMPLE_SIZE)
    console.log(`Pill brightness: before=${b0.toFixed(1)}, after=${b1.toFixed(1)}`)

    // Hover should produce a visibly brighter pill
    expect(b1).toBeGreaterThan(b0 + 3)
})
