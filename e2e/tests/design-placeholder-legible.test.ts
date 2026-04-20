import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, peakBrightness } from "../fixtures/pixels"

// Placeholder hint text must be clearly legible against the dark card fill.
// The card is dark (~25 brightness); default egui hint color washes out
// ("шрифты слишком серые"). We check PEAK brightness inside the glyph
// area — antialiased text has thin strokes so average is dominated by
// background, but the peak catches whether the stroke itself is bright.
test("placeholder hint text is legible", async ({ nudge }) => {
    await new Promise((r) => setTimeout(r, 700))

    const buf = (await nudge.page.screenshot()) as Buffer
    const png = decode(buf)

    // Row 2 ("Хуйня?") is unfocused at first run → hint text visible.
    // Card left ≈ 160; glyphs start ~20 px in, so band is x≈180..270, y≈230..250.
    const peak = peakBrightness(png, 180, 230, 100, 20)
    console.log(`placeholder-hint peak: ${peak.toFixed(1)}`)

    // Legible anti-aliased glyphs should peak ≥140. Default dim hint peaks ~100.
    expect(peak).toBeGreaterThan(140)
})
