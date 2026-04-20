import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("three equal-height rows with placeholder-as-label (no external labels) and thin dividers between rows", async ({ nudge }) => {
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The card shows exactly three stacked rows of equal height. Row 1 contains greyed hint text starting with the Russian word 'Что'. Row 2 contains short greyed Russian hint text. Row 3 contains the digits '10'. The three rows are separated by visible thin horizontal lines. Each row occupies a single line of height — there are NOT two lines per row (one label + one empty field)."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
