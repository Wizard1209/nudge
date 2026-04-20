import { expect } from "vitest"
import { test } from "../fixtures/nudge"

// Spec: next_interval_minutes must be > 0. Entering a non-positive number
// and pressing Enter is a validation error — the popup must stay open and
// nothing must be written to the journal.
test("negative interval rejects submit and writes nothing", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // Focus the minutes field (3rd row, y ≈ 285), select-all, type "-5".
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.down("Control")
    await nudge.page.keyboard.press("a")
    await nudge.page.keyboard.up("Control")
    await nudge.page.keyboard.type("-5", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))

    // Attempt to submit.
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Journal must remain empty — invalid interval blocks the write.
    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(journal ?? "").toBe("")

    // Popup stays open: re-focus the minutes field, fix the value, submit —
    // that second submit must succeed and produce exactly one journal entry.
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.down("Control")
    await nudge.page.keyboard.press("a")
    await nudge.page.keyboard.up("Control")
    await nudge.page.keyboard.type("5", { delay: 30 })
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 600))

    const after = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(after).not.toBeNull()
    const lines = after!.split("\n").filter((l) => l.length > 0)
    expect(lines.length).toBe(1)
    const entry = JSON.parse(lines[0])
    expect(entry.next_interval_minutes).toBe(5)
})
