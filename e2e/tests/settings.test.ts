import { expect } from "vitest"
import { test } from "../fixtures/settings"

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

const ROW_HOTKEY_Y = 43
const ROW_INTERVAL_Y = 145
const ROW_AUTOSTART_Y = 190
const BUTTON_ROW_Y = 488

const HOTKEY_INPUT_X = 667
const INTERVAL_INPUT_X = 700
const AUTOSTART_CHECKBOX_X = 758
const SAVE_BUTTON_X = 744

async function clickAt(page: Awaited<ReturnType<typeof test.extend<unknown>["page"]>>, x: number, y: number): Promise<void> {
    await page.mouse.click(x, y)
    await wait(200)
    await page.mouse.click(x, y)
    await wait(200)
}

async function readPersistedConfig(page: any): Promise<{
    hotkey?: string
    default_interval_minutes?: number
    autostart?: boolean
} | null> {
    const raw = await page.evaluate(() => localStorage.getItem("nudge-config"))
    if (raw == null) return null
    return JSON.parse(raw)
}

async function retypeInterval(page: any, text: string): Promise<void> {
    await clickAt(page, INTERVAL_INPUT_X, ROW_INTERVAL_Y)
    await page.keyboard.down("Control")
    await page.keyboard.press("a")
    await page.keyboard.up("Control")
    await page.keyboard.type(text, { delay: 30 })
    await wait(200)
}

test("settings page boots and writes the whole config on the first Save", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await retypeInterval(settings.page, "15")
    await settings.page.mouse.click(SAVE_BUTTON_X, BUTTON_ROW_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.hotkey).toBe("Ctrl+Shift+Space")
    expect(persisted!.default_interval_minutes).toBe(15)
    expect(persisted!.autostart).toBe(false)
})

test("Return in a field saves", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await retypeInterval(settings.page, "7")
    await settings.page.keyboard.press("Enter")
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.default_interval_minutes).toBe(7)
})

test("Return on an unedited form writes nothing", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await clickAt(settings.page, INTERVAL_INPUT_X, ROW_INTERVAL_Y)
    await settings.page.keyboard.press("Enter")
    await wait(500)

    expect(await readPersistedConfig(settings.page)).toBeNull()
})

test("editing interval and saving persists the new value", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await retypeInterval(settings.page, "7")

    await settings.page.mouse.click(SAVE_BUTTON_X, BUTTON_ROW_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.default_interval_minutes).toBe(7)
})

test("autostart toggle persists immediately, without Save", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await settings.page.mouse.click(AUTOSTART_CHECKBOX_X, ROW_AUTOSTART_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.autostart).toBe(true)
})
