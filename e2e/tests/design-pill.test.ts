import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("after dismiss, only a small pill in the bottom-right shows M:SS countdown", async ({ nudge }) => {
    // Dismiss via Esc
    await nudge.page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 600))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The card with input fields is NOT visible. In the bottom-right region of the window there is a small pill-shaped element (a rounded rectangle, much smaller than the card) showing a short time in 'M:SS' format like '9:59' or '10:00'. There is NO big centered 'Next nudge in' text, NO 'Nudge now' button, NO large panel — the pill is the only non-wallpaper element on screen."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
