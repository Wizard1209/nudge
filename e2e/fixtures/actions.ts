import type { Page } from "puppeteer"

// Canvas-relative coordinates for the three input rows in the 800×600 viewport.
// Row 1 = "Что я делаю?" (auto-focused), Row 2 = "Хуйня?" (Tab away from Row 1),
// Row 3 = minutes. Tied to draw_card's top_offset = 0.25 * 600 = 150 (spec §1).
export const FIELD_DOING_Y = 170
export const FIELD_BULLSHIT_Y = 212
export const FIELD_MINUTES_Y = 254
export const FIELD_X = 400

// Bottom-right pill centre. Pill ≈ 60×30 at the bottom-right corner with ~16px
// inset; sampled experimentally in design-pill-hover.
export const PILL_X = 755
export const PILL_Y = 570

export const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

/** Double-click a canvas point to give egui's text edit reliable focus. */
export async function focusField(page: Page, y: number, x = FIELD_X): Promise<void> {
    await page.mouse.click(x, y)
    await wait(250)
    await page.mouse.click(x, y)
    await wait(250)
}

/** Select-all + type — for replacing the contents of an already-focused field. */
export async function selectAllAndType(page: Page, text: string): Promise<void> {
    await page.keyboard.down("Control")
    await page.keyboard.press("a")
    await page.keyboard.up("Control")
    await page.keyboard.type(text, { delay: 30 })
    await wait(200)
}

/** Click the bottom-right pill to re-open the form. */
export async function clickPill(page: Page): Promise<void> {
    await page.mouse.click(PILL_X, PILL_Y)
    await wait(700)
}

export type DismissTrigger = "esc" | "click-outside" | "blur"

/** Dismiss the popup via the given trigger and wait for the hide animation. */
export async function dismissForm(page: Page, via: DismissTrigger = "esc"): Promise<void> {
    if (via === "esc") {
        await page.keyboard.press("Escape")
        await wait(600)
    } else if (via === "click-outside") {
        await page.mouse.click(10, 10)
        await wait(600)
    } else {
        // Drive `document.hasFocus()` directly. The app polls it each frame,
        // so flipping the return value is what actually triggers switch-away.
        // (Dispatching a synthetic FocusEvent would not change hasFocus and
        // therefore would not exercise the production path.)
        await page.evaluate(() => {
            Object.defineProperty(document, "hasFocus", {
                configurable: true,
                value: () => false,
            })
        })
        await wait(1500)
    }
}
