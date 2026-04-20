import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("clicking outside the card dismisses the popup without writing a journal entry", async ({ nudge }) => {
    // Clear any prior journal state
    await nudge.page.evaluate(() => localStorage.clear())

    // Type into first field (auto-focused)
    await nudge.page.keyboard.type("should NOT save", { delay: 30 })
    await new Promise((r) => setTimeout(r, 300))

    // Click outside the card — top-left corner of the viewport
    await nudge.page.mouse.click(10, 10)
    await new Promise((r) => setTimeout(r, 500))

    // Popup must be dismissed (card gone)
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The input form card is NOT visible. The only visible UI element (aside from the wallpaper) is a countdown or a small pill, NOT a rounded card with stacked input rows."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)

    // Journal must have no entry containing the typed text
    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal after click-outside:", journal)
    expect(journal ?? "").not.toContain("should NOT save")
})
