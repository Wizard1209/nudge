import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("card is a distinct floating surface with rounded corners, translucent fill, and soft shadow over a visible wallpaper", async ({ nudge }) => {
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The form is rendered inside a distinctly-shaped card that sits as a floating rectangle with clearly rounded corners and a soft shadow. The card does NOT fill the entire window — there is a visible wallpaper or background area outside the card's borders (e.g. a gradient, pattern, or image surrounding the card). The card's fill is dark but translucent enough that it reads as a 'frosted glass' / spotlight-style surface rather than a flat opaque rectangle."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})
