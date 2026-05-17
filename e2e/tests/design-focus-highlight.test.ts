import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"
import { focusField, wait, FIELD_DOING_Y } from "../fixtures/actions"

// Row centres at 800×600 viewport. Each row is 40px tall + 1px divider.
// Tied to draw_card top_offset = 0.25 * 600 = 150 (spec §1).
const ROW1_Y = 170
const ROW2_Y = 212
const ROW3_Y = 254

// Three brightness asserts on a SINGLE screenshot taken after Tab into row 2.
// Each assert protects a separate regression:
//   1. visible       — focused row is brighter than unfocused rows
//   2. subtle        — focused row is NOT a solid white block
//   3. inset         — highlight stops short of the card's left edge
test("focused row is visibly highlighted, subtle, and inset from card edges", async ({ nudge }) => {
    await nudge.page.bringToFront()
    // Click into row 1 so the canvas receives focus, then Tab to row 2.
    await focusField(nudge.page, FIELD_DOING_Y)
    await nudge.page.keyboard.press("Tab")
    await wait(900)

    const buf = (await nudge.page.screenshot()) as Buffer
    const png = decode(buf)

    // ─── 1. Visible: row 2 background brighter than rows 1 & 3 ───
    // Sample a clean band on the right of each row, away from text/caret.
    const SAMPLE_X = 560
    const SAMPLE_W = 40
    const SAMPLE_H = 20
    const row1 = avgBrightness(png, SAMPLE_X, ROW1_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    const row2 = avgBrightness(png, SAMPLE_X, ROW2_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    const row3 = avgBrightness(png, SAMPLE_X, ROW3_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    console.log(`visible: r1=${row1.toFixed(1)} r2=${row2.toFixed(1)} r3=${row3.toFixed(1)}`)
    expect(row2).toBeGreaterThan(row1 + 3)
    expect(row2).toBeGreaterThan(row3 + 3)

    // ─── 2. Subtle: row 2 stays below "solid white block" threshold ───
    // Sample mid-row where the highlight overlay has full coverage.
    const MID_X = 380
    const MID_W = 60
    const MID_H = 16
    const row2Mid = avgBrightness(png, MID_X, ROW2_Y - MID_H / 2, MID_W, MID_H)
    const row1Mid = avgBrightness(png, MID_X, ROW1_Y - MID_H / 2, MID_W, MID_H)
    console.log(`subtle: r1Mid=${row1Mid.toFixed(1)} r2Mid=${row2Mid.toFixed(1)} diff=${(row2Mid - row1Mid).toFixed(1)}`)
    expect(row2Mid).toBeLessThan(55)
    expect(row2Mid - row1Mid).toBeLessThan(25)

    // ─── 3. Inset: highlight stops short of card's left edge ───
    // Card left ≈ x=160. Gutter band (~4 px in) should still show card fill,
    // not the highlight tint.
    const GUTTER_X = 163
    const GUTTER_W = 8
    const INSIDE_X = 280
    const INSIDE_W = 40
    const INSIDE_H = 12
    const gutter = avgBrightness(png, GUTTER_X, ROW2_Y - INSIDE_H / 2, GUTTER_W, INSIDE_H)
    const inside = avgBrightness(png, INSIDE_X, ROW2_Y - INSIDE_H / 2, INSIDE_W, INSIDE_H)
    console.log(`inset: gutter=${gutter.toFixed(1)} inside=${inside.toFixed(1)}`)
    expect(inside).toBeGreaterThan(gutter + 5)
})
