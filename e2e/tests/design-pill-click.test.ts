import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("clicking the bottom-right pill re-opens the input form", async ({ nudge }) => {
    // Hide the popup by clicking outside the card (top-left corner). Avoids
    // relying on keyboard.press — in a full-suite run that one is flaky in
    // headless:false WSL Chrome (zombie windows from earlier tests steal
    // keyboard focus from the new browser instance).
    await nudge.page.mouse.click(10, 10)
    await new Promise((r) => setTimeout(r, 600))

    // Click on the pill — bottom-right with ~16px margin, pill ≈ 60×30
    await nudge.page.mouse.click(755, 570)
    await new Promise((r) => setTimeout(r, 1000))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A rounded dark card with stacked input rows is visible in the upper half of the window. The small bottom-right pill is NOT visible anymore."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
