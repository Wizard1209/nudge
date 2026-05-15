import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("losing window focus hides popup but preserves typed text for next open", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // Click inside the card so the canvas becomes DOM-focused, then type.
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("preserved across blur", { delay: 30 })
    await new Promise((r) => setTimeout(r, 300))

    // Simulate window focus loss
    await nudge.page.evaluate(() => {
        window.dispatchEvent(new FocusEvent("blur"))
    })
    await new Promise((r) => setTimeout(r, 600))

    // Card must be HIDDEN
    const hiddenShot = await nudge.page.screenshot()
    const hiddenJudge = await visualAssert(
        hiddenShot as Buffer,
        "The input form card is NOT visible. The only visible UI element (aside from the wallpaper) is a small countdown pill in the bottom-right corner."
    )
    console.log("Judge (hidden):", hiddenJudge)
    expect(hiddenJudge.pass).toBe(true)

    // Journal must still be empty
    let journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(journal ?? "").not.toContain("preserved across blur")

    // Re-open via pill click
    await nudge.page.mouse.click(755, 570)
    await new Promise((r) => setTimeout(r, 500))

    // Press Enter — if text was preserved, journal entry contains it
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal after reopen+Enter:", journal)
    expect(journal ?? "").toContain("preserved across blur")
})
