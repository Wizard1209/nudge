import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"

// The focus highlight must be INSET from the card edges — not a flat
// rectangle touching the card's rounded corners / stroke. User-visible bug:
// "прямоугольник выходит за края". A properly inset tint leaves a narrow
// card-coloured gutter on the left/right of the focused row.
//
// Approach: Tab into row 2 (middle row, not affected by rounded corners),
// then compare mid-row brightness at two x-positions:
//   - GUTTER: ~4 px inside the card's left edge → should match card fill
//   - INSIDE: well inside the card → should show the highlight tint
test("focus highlight is inset from card edges", async ({ nudge }) => {
    await nudge.page.bringToFront()
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.keyboard.press("Tab")
    await new Promise((r) => setTimeout(r, 800))

    const buf = (await nudge.page.screenshot()) as Buffer
    const png = decode(buf)

    // Card left ≈ x=160 at 800×600. Sample GUTTER at x=162..170 and
    // INSIDE at x=280..320, both on the focused row 2 band.
    const GUTTER_X = 163
    const GUTTER_W = 8
    const INSIDE_X = 280
    const INSIDE_W = 40
    const Y = 236
    const H = 12

    const gutter = avgBrightness(png, GUTTER_X, Y, GUTTER_W, H)
    const inside = avgBrightness(png, INSIDE_X, Y, INSIDE_W, H)
    console.log(`inset-check: gutter=${gutter.toFixed(1)} inside=${inside.toFixed(1)}`)

    // Inside must be brighter than the gutter — i.e. highlight stops short
    // of the card edge. Tolerate noise with a 5-unit threshold.
    expect(inside).toBeGreaterThan(gutter + 5)
})
