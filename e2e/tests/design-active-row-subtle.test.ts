import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"

// Row centers match design-active-row.test.ts so we stay comparable.
const ROW1_Y = 200
const ROW2_Y = 242
// Sample mid-row where the row_field's rect_filled overlay has full coverage
// (not near rounded corners which average in the dark card fill).
const SAMPLE_X = 380
const SAMPLE_W = 60
const SAMPLE_H = 16

// Focused-row highlight must be SUBTLE — not a solid light/white block.
// The spec calls for "a slightly brighter background"; on the current code
// the overlay lands on top of a semi-transparent card and the compound
// blend makes the focused row ~3× brighter than an unfocused one. That is
// the bug the user reported ("белый прямоугольник ложащийся очень не аккуратно").
test("focused row highlight is subtle, not a bright block", async ({ nudge }) => {
    await nudge.page.bringToFront()
    await nudge.page.mouse.move(400, 200)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.keyboard.press("Tab")
    await new Promise((r) => setTimeout(r, 1000))

    const buf = (await nudge.page.screenshot()) as Buffer
    const png = decode(buf)

    const row1 = avgBrightness(png, SAMPLE_X, ROW1_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    const row2 = avgBrightness(png, SAMPLE_X, ROW2_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    console.log(`subtle-check: r1=${row1.toFixed(1)} r2=${row2.toFixed(1)} diff=${(row2 - row1).toFixed(1)}`)

    // Absolute ceiling — catches the native "solid white row" regression
    // where brightness jumps to ~200+. Card fill is ~25, a subtle tint
    // should stay below ~55.
    expect(row2).toBeLessThan(55)

    // Relative ceiling — the difference between focused and unfocused rows
    // should be visually noticeable but not harsh. Current buggy value is
    // ~54 (nearly 3× the card fill); ~15 is roughly the target.
    expect(row2 - row1).toBeLessThan(25)
})
