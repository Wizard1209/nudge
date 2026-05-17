import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { focusField, selectAllAndType, wait, FIELD_MINUTES_Y } from "../fixtures/actions"

// Spec: next_interval_minutes must be > 0. Entering a non-positive number
// and pressing Enter is a validation error — nothing must be written.
// (Recovery via valid resubmit is covered by form.test.ts > Enter saves journal entry.)
test("negative interval rejects submit and writes nothing", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    await focusField(nudge.page, FIELD_MINUTES_Y)
    await selectAllAndType(nudge.page, "-5")

    await nudge.page.keyboard.press("Enter")
    await wait(500)

    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    expect(journal ?? "").toBe("")
})
