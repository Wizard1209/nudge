import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert, PROMPT_CARD_PRESENT } from "../fixtures/judge"
import { focusField, selectAllAndType, wait, FIELD_MINUTES_Y } from "../fixtures/actions"

test("Timer auto-triggers form reappearance", async ({ nudge }) => {
    // Set minutes to 0.02 (~1.2 s timer). Spec forbids non-positive.
    await focusField(nudge.page, FIELD_MINUTES_Y)
    await selectAllAndType(nudge.page, "0.02")

    // Submit — form hides, short timer starts.
    await nudge.page.keyboard.press("Enter")
    await wait(500)

    // Wait for timer to expire + re-render.
    await wait(3000)

    const screenshot = (await nudge.page.screenshot()) as Buffer
    const result = await visualAssert(screenshot, PROMPT_CARD_PRESENT)
    console.log("Form reappeared:", result)
    expect(result.pass).toBe(true)
})
