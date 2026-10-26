import { expect } from "vitest"
import { test } from "../fixtures/settings"

/**
 * Settings UI e2e tests — drive the same lib.rs `wasm_entry::start` that
 * native uses for the SettingsApp branch (URL contains "settings"). All
 * three settings — interval, hotkey, autostart — must round-trip through
 * localStorage under the `nudge-config` key, matching what the Rust
 * `config::save_to_localstorage` writes and what `config::load_from_localstorage`
 * would read on the next boot.
 *
 * Coordinates below are for the 800×600 viewport. The SettingsApp lays the
 * rows out in groups (hotkey with its footnote under it; interval and
 * autostart; the sprite rows), each row a label on the left and its control
 * at the right edge, then a reserved message line and Cancel, Save at the
 * bottom right. The groups stretch to the canvas width, so the controls sit
 * near x≈780 here rather than at the native window's right edge.
 */

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

// Centres of the controls, read off a screenshot of the rendered canvas
// (e2e/dist served at localhost:8080, the fixture's 800x600 at 1x): hotkey
// field x 567-766 / y 34-52, interval field x 665-736 / y 136-154, autostart
// box x 751-764 / y 183-196, Save x 708-779 / y 473-502. The buttons follow
// the form, they are not anchored to the canvas bottom.
const ROW_HOTKEY_Y = 43
const ROW_INTERVAL_Y = 145
const ROW_AUTOSTART_Y = 190
const BUTTON_ROW_Y = 488

const HOTKEY_INPUT_X = 667
const INTERVAL_INPUT_X = 700
// The autostart checkbox sits at the right end of its row.
const AUTOSTART_CHECKBOX_X = 758
// Save is the filled button, right of Cancel.
const SAVE_BUTTON_X = 744

async function clickAt(page: Awaited<ReturnType<typeof test.extend<unknown>["page"]>>, x: number, y: number): Promise<void> {
    // Double click → reliably gives egui's TextEdit focus (see actions.ts).
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

/** Put the caret in the interval field and replace its text. */
async function retypeInterval(page: any, text: string): Promise<void> {
    await clickAt(page, INTERVAL_INPUT_X, ROW_INTERVAL_Y)
    await page.keyboard.down("Control")
    await page.keyboard.press("a")
    await page.keyboard.up("Control")
    await page.keyboard.type(text, { delay: 30 })
    await wait(200)
}

test("settings page boots and writes the whole config on the first Save", async ({ settings }) => {
    // Fresh load: localStorage is empty, the form's baseline is Config::default().
    await settings.page.evaluate(() => localStorage.clear())

    // One edit, then Save: §9 greys Save while the form matches the file, so
    // a no-edit click would write nothing. The write carries the whole
    // config, defaults included. The key "nudge-config" is the contract the
    // native config::load_from_localstorage reads back.
    await retypeInterval(settings.page, "15")
    await settings.page.mouse.click(SAVE_BUTTON_X, BUTTON_ROW_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.hotkey).toBe("Ctrl+Shift+Space")
    expect(persisted!.default_interval_minutes).toBe(15)
    expect(persisted!.autostart).toBe(false)
})

// §9 Saving: "Return anywhere in the form is `Save`: the same checks, the
// same write, the same messages as the button."
test("Return in a field saves", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await retypeInterval(settings.page, "7")
    await settings.page.keyboard.press("Enter")
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.default_interval_minutes).toBe(7)
})

// §9 Saving: "`Save` is live only while the form differs from the file".
test("Return on an unedited form writes nothing", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    await clickAt(settings.page, INTERVAL_INPUT_X, ROW_INTERVAL_Y)
    await settings.page.keyboard.press("Enter")
    await wait(500)

    expect(await readPersistedConfig(settings.page)).toBeNull()
})

test("editing interval and saving persists the new value", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    // Focus interval field, replace contents with 7, Save.
    await retypeInterval(settings.page, "7")

    await settings.page.mouse.click(SAVE_BUTTON_X, BUTTON_ROW_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.default_interval_minutes).toBe(7)
})

test("autostart toggle persists immediately, without Save", async ({ settings }) => {
    // The autostart checkbox routes through the transactional rule the
    // moment it's clicked (apply_autostart succeeds-then-persists). No
    // Save click — proves the FakeProvider + persistence wiring works.
    await settings.page.evaluate(() => localStorage.clear())

    await settings.page.mouse.click(AUTOSTART_CHECKBOX_X, ROW_AUTOSTART_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.autostart).toBe(true)
})
