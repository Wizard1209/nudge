import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("fresh page load shows the input form card, not the countdown/pill", async ({ nudge }) => {
    // fixtures/nudge waits 1s for initial paint; no user interaction required.
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A rounded dark card with stacked input rows is visible in the upper half of the window. There is NO big 'Next nudge in' countdown text and NO 'Nudge now' button anywhere in the screenshot."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
