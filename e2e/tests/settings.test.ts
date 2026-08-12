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
 * spacing. The SettingsApp renders a heading, then three rows with ~24px
 * each, then a banner area, then the button row.
 */

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))

// Approximate Y-coordinates for the SettingsApp rows under egui's default
// spacing in the 800x600 viewport. Calibrated against a screenshot of the
// rendered canvas (e2e/dist served at localhost:8080). Loosen by ±5px if
// egui's default spacing changes upstream.
const ROW_HOTKEY_Y = 42
const ROW_INTERVAL_Y = 68
const ROW_AUTOSTART_Y = 97
const BUTTON_ROW_Y = 130

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

test("settings page boots and seeds localStorage on Save", async ({ settings }) => {
    // Fresh load: localStorage is empty, the form's baseline is Config::default().
    await settings.page.evaluate(() => localStorage.clear())

    // Click Save once with no edits — this exercises the save path end-to-end
    // and writes the default config to localStorage. The key "nudge-config"
    // is the contract the native config::load_from_localstorage reads back.
    await settings.page.mouse.click(SAVE_BUTTON_X, BUTTON_ROW_Y)
    await wait(500)

    const persisted = await readPersistedConfig(settings.page)
    expect(persisted).not.toBeNull()
    expect(persisted!.hotkey).toBe("Ctrl+Shift+Space")
    expect(persisted!.default_interval_minutes).toBe(10)
    expect(persisted!.autostart).toBe(false)
})

test("editing interval and saving persists the new value", async ({ settings }) => {
    await settings.page.evaluate(() => localStorage.clear())

    // Focus interval field, replace contents with 7, Save.
    await clickAt(settings.page, INTERVAL_INPUT_X, ROW_INTERVAL_Y)
    await settings.page.keyboard.down("Control")
    await settings.page.keyboard.press("a")
    await settings.page.keyboard.up("Control")
    await settings.page.keyboard.type("7", { delay: 30 })
    await wait(200)

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
