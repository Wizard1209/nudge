import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("clicking outside hides popup but preserves typed text for next open", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // Type into first field (auto-focused)
    await nudge.page.keyboard.type("preserved across switch", { delay: 30 })
    await new Promise((r) => setTimeout(r, 300))

    // Click outside the card — top-left corner of the viewport
    await nudge.page.mouse.click(10, 10)
    await new Promise((r) => setTimeout(r, 500))

    // Card must be HIDDEN; pill must be visible
    const hiddenShot = await nudge.page.screenshot()
    const hiddenJudge = await visualAssert(
        hiddenShot as Buffer,
        "The input form card is NOT visible. The only visible UI element (aside from the wallpaper) is a small countdown pill in the bottom-right corner."
    )
    console.log("Judge (hidden):", hiddenJudge)
    expect(hiddenJudge.pass).toBe(true)

    // Journal must still be empty — switch does not write
    let journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(journal ?? "").not.toContain("preserved across switch")

    // Re-open via pill click
    await nudge.page.mouse.click(755, 570)
    await new Promise((r) => setTimeout(r, 500))

    // Press Enter — if the text was preserved, it should land in the journal
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal after reopen+Enter:", journal)
    expect(journal ?? "").toContain("preserved across switch")
})
