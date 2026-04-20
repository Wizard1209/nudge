import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("losing window focus dismisses the popup without writing a journal entry", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // Click inside the card so the canvas becomes DOM-focused, then type.
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("focus-loss text", { delay: 30 })
    await new Promise((r) => setTimeout(r, 300))

    // Simulate window focus loss: fire a real blur event on the window object.
    // The app listens for this via a DOM blur listener installed from Rust.
    await nudge.page.evaluate(() => {
        window.dispatchEvent(new FocusEvent("blur"))
    })
    await new Promise((r) => setTimeout(r, 600))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The input form card is NOT visible. The only visible UI element (aside from the wallpaper) is a countdown or a small pill, NOT a rounded card with stacked input rows."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)

    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(journal ?? "").not.toContain("focus-loss text")
})
