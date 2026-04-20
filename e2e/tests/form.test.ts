import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("can type into the first field", async ({ nudge }) => {
    // Click on the first text field (with dark theme padding: field at ~y=55)
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 500))

    // egui needs a second click sometimes to activate text edit
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 300))

    // Type using keyboard.type — sends keydown/keypress/keyup for each char
    await nudge.page.keyboard.type("hello nudge", { delay: 50 })
    await new Promise((r) => setTimeout(r, 500))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The text 'hello nudge' is visible in a text input field on the screen"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Enter hides form and shows the bottom-right pill", async ({ nudge }) => {
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 1000))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The input card is NOT visible. In the bottom-right region a small pill shows a short time in 'M:SS' format. There is NO large centered 'Next nudge in' text and NO 'Nudge now' button."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Esc hides form and shows the bottom-right pill", async ({ nudge }) => {
    await nudge.page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 1000))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "The input card is NOT visible. In the bottom-right region a small pill shows a short time in 'M:SS' format. There is NO large centered 'Next nudge in' text and NO 'Nudge now' button."
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Timer auto-triggers form reappearance", async ({ nudge }) => {
    // Click on the minutes field and change to "0.02" (~1.2 s timer — must
    // be > 0 because the journal spec forbids non-positive intervals).
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 300))

    await nudge.page.keyboard.down("Control")
    await nudge.page.keyboard.press("a")
    await nudge.page.keyboard.up("Control")
    await nudge.page.keyboard.type("0.02", { delay: 50 })
    await new Promise((r) => setTimeout(r, 300))

    // Submit — form hides, timer starts with 1-second interval
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Wait for 1-second timer to expire + re-render
    await new Promise((r) => setTimeout(r, 3000))

    // Form should have reappeared automatically
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A rounded dark card with stacked input rows is visible in the upper half of the window. There is NO big 'Next nudge in' countdown text and NO 'Nudge now' button anywhere in the screenshot."
    )
    console.log("Form reappeared:", result)
    expect(result.pass).toBe(true)
})

test("Enter saves journal entry to localStorage", async ({ nudge }) => {
    // Clear localStorage first
    await nudge.page.evaluate(() => localStorage.clear())

    // Type into fields
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("writing code", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))

    // Tab to second field
    await nudge.page.keyboard.press("Tab")
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("no", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))

    // Submit
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Read localStorage
    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal:", journal)

    expect(journal).not.toBeNull()
    const lines = journal!.split("\n")
    expect(lines.length).toBe(1)

    const entry = JSON.parse(lines[0])
    expect(entry.schema_version).toBe(1)
    expect(entry.event_type).toBe("submitted")
    expect(entry.entry_id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/) // ULID
    expect(entry.captured_at).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}[+-]\d{2}:\d{2}$/)
    expect(entry.implementation).toBe("rust")
    // Initial popup on page load is timer-triggered by default
    expect(entry.trigger_source).toBe("timer")
    expect(entry.doing).toBe("writing code")
    expect(entry.bullshit).toBe("no")
    expect(entry.next_interval_minutes).toBe(10)
})

test("Esc does NOT save journal entry", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // Type something
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("should not save", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))

    // Dismiss with Esc
    await nudge.page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 500))

    // localStorage should be empty
    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal after Esc:", journal)
    expect(journal).toBeNull()
})

test("Multiple submits append to journal (timer-driven reopen)", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // First entry — set minutes to 0.02 (~1.2 s) so timer re-fires quickly
    // after submit. Zero is rejected now (spec requires positive interval).
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.mouse.click(400, 285)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.down("Control")
    await nudge.page.keyboard.press("a")
    await nudge.page.keyboard.up("Control")
    await nudge.page.keyboard.type("0.02", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))

    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("first entry", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.press("Enter")

    // Wait for timer auto-reopen (interval ≈ 1s)
    await new Promise((r) => setTimeout(r, 2500))

    // Second entry
    await nudge.page.mouse.click(400, 200)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("second entry", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Should have 2 NDJSON entries (no header)
    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal:", journal)

    expect(journal).not.toBeNull()
    const lines = journal!.split("\n")
    expect(lines.length).toBe(2)

    const first = JSON.parse(lines[0])
    const second = JSON.parse(lines[1])
    expect(first.doing).toBe("first entry")
    expect(second.doing).toBe("second entry")
    expect(first.entry_id).not.toBe(second.entry_id)
})
