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
 * Coordinates below are tuned for the 800×600 viewport and egui's default
 * spacing. The SettingsApp renders a heading, then the rows with ~24px
 * each — the hotkey row followed by its vocabulary line (design-spec §9),
 * interval, autostart, then the sprite rows — then a banner area, then the
 * button row.
 */

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

// Approximate Y-coordinates for the SettingsApp rows under egui's default
// spacing in the 800x600 viewport. Calibrated against a screenshot of the
// rendered canvas (e2e/dist served at localhost:8080). Loosen by ±5px if
// egui's default spacing changes upstream. Everything below the hotkey row
// sits one small-text line (~14px) lower than it would without the
// vocabulary line under the hotkey field.
const ROW_HOTKEY_Y = 42
const ROW_INTERVAL_Y = 82
const ROW_AUTOSTART_Y = 110
// The sprite rows (checkbox, PNG path, X / Y / card width) sit between the
// autostart row and the buttons, so the button row is three rows lower
// than the autostart one.
const BUTTON_ROW_Y = 214

// The hotkey + interval TextEdits sit to the right of their labels; X
// coordinates target the middle of each input box.
const HOTKEY_INPUT_X = 237
const INTERVAL_INPUT_X = 256
// Checkbox glyph for autostart sits at the very left of its row.
const AUTOSTART_CHECKBOX_X = 9
// Save button is the first (leftmost) button in the bottom row.
const SAVE_BUTTON_X = 36

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
