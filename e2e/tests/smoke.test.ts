import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("shows Nudge works heading", async ({ nudge }) => {
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A heading with text 'Nudge works!' is visible on the screen"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
