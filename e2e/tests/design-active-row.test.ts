import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"

// Approximate row centers (card at y≈180, each row 40px high + 1px divider)
const ROW1_Y = 200
const ROW2_Y = 242
const ROW3_Y = 284
// Sample an empty band to the right of any text (avoid hint text / caret pixels).
const SAMPLE_X = 560
const SAMPLE_W = 40
const SAMPLE_H = 20

test("focused row has a brighter background than unfocused rows", async ({ nudge }) => {
    // Ensure window has input focus so keys reach egui, then Tab into row 2.
    await nudge.page.bringToFront()
    await nudge.page.mouse.move(400, 200)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.keyboard.press("Tab")
    await new Promise((r) => setTimeout(r, 1000))

    const buf = (await nudge.page.screenshot()) as Buffer
    const png = decode(buf)

    const row1 = avgBrightness(png, SAMPLE_X, ROW1_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    const row2 = avgBrightness(png, SAMPLE_X, ROW2_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    const row3 = avgBrightness(png, SAMPLE_X, ROW3_Y - SAMPLE_H / 2, SAMPLE_W, SAMPLE_H)
    console.log(`Row brightness: r1=${row1.toFixed(1)}, r2=${row2.toFixed(1)}, r3=${row3.toFixed(1)}`)

    // Focused row should be measurably brighter than both unfocused rows
    expect(row2).toBeGreaterThan(row1 + 3)
    expect(row2).toBeGreaterThan(row3 + 3)
})
