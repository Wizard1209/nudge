import type { Page } from "puppeteer"

export const FIELD_DOING_Y = 170
export const FIELD_BULLSHIT_Y = 212
export const FIELD_MINUTES_Y = 254
export const FIELD_X = 400

export const PILL_X = 755
export const PILL_Y = 570

export const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

export async function focusField(page: Page, y: number, x = FIELD_X): Promise<void> {
    await page.mouse.click(x, y)
    await wait(250)
    await page.mouse.click(x, y)
    await wait(250)
}

export async function selectAllAndType(page: Page, text: string): Promise<void> {
    await page.keyboard.down("Control")
    await page.keyboard.press("a")
    await page.keyboard.up("Control")
    await page.keyboard.type(text, { delay: 30 })
    await wait(200)
}

export async function clickPill(page: Page): Promise<void> {
    await page.mouse.click(PILL_X, PILL_Y)
    await wait(700)
}

export type DismissTrigger = "esc" | "click-outside" | "blur"

export async function dismissForm(page: Page, via: DismissTrigger = "esc"): Promise<void> {
    if (via === "esc") {
        await page.keyboard.press("Escape")
        await wait(600)
    } else if (via === "click-outside") {
        await page.mouse.click(10, 10)
        await wait(600)
    } else {
        await page.evaluate(() => {
            Object.defineProperty(document, "hasFocus", {
                configurable: true,
                value: () => false,
            })
        })
        await wait(1500)
    }
}
