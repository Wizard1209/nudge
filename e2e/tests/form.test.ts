import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { visualAssert } from "../fixtures/judge"

test("shows three labeled input fields", async ({ nudge }) => {
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "Three labeled text input fields are visible: one labeled 'Что я делаю?', one labeled 'Не хуйню ли я делаю?', and one labeled with minutes/interval showing the value '10'"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("form has polished spotlight appearance", async ({ nudge }) => {
    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A dark-themed form with visible padding/margins around the content (not touching the edges), the text fields are wide (spanning most of the width), and the overall look is clean and polished — NOT a raw unstyled form crammed into the top-left corner"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("can type into the first field", async ({ nudge }) => {
    // Click on the first text field (with dark theme padding: field at ~y=55)
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 500))

    // egui needs a second click sometimes to activate text edit
    await nudge.page.mouse.click(400, 55)
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

test("Enter hides form and shows countdown screen", async ({ nudge }) => {
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 1000))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A countdown screen showing text 'Next nudge in' followed by a time like M:SS or MM:SS, and a button labeled 'Nudge now'"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Esc hides form and shows countdown screen", async ({ nudge }) => {
    await nudge.page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 1000))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "A countdown screen showing text 'Next nudge in' followed by a time, and a button labeled 'Nudge now'"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Nudge now button re-shows form", async ({ nudge }) => {
    // First hide the form
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 1000))

    // Click the "Nudge now" button (centered horizontally, at ~y=115)
    await nudge.page.mouse.click(400, 120)
    await new Promise((r) => setTimeout(r, 500))

    const screenshot = await nudge.page.screenshot()
    const result = await visualAssert(
        screenshot as Buffer,
        "Three labeled text input fields are visible (a form, not a countdown screen)"
    )
    console.log("Judge says:", result)
    expect(result.pass).toBe(true)
})

test("Timer auto-triggers form reappearance", async ({ nudge }) => {
    // Click on the minutes field and change to "0" (becomes 1-second timer)
    await nudge.page.mouse.click(400, 180)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 180)
    await new Promise((r) => setTimeout(r, 300))

    // Select all and replace with "0"
    await nudge.page.keyboard.down("Control")
    await nudge.page.keyboard.press("a")
    await nudge.page.keyboard.up("Control")
    await nudge.page.keyboard.type("0", { delay: 50 })
    await new Promise((r) => setTimeout(r, 300))

    // Submit — form hides, timer starts with 1-second interval
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Verify countdown screen
    let screenshot = await nudge.page.screenshot()
    let result = await visualAssert(
        screenshot as Buffer,
        "A countdown screen with 'Next nudge in' text and a 'Nudge now' button"
    )
    console.log("Countdown visible:", result)
    expect(result.pass).toBe(true)

    // Wait for 1-second timer to expire + re-render
    await new Promise((r) => setTimeout(r, 3000))

    // Form should have reappeared automatically
    screenshot = await nudge.page.screenshot()
    result = await visualAssert(
        screenshot as Buffer,
        "Three labeled text input fields are visible (a form with text inputs, not a countdown)"
    )
    console.log("Form reappeared:", result)
    expect(result.pass).toBe(true)
})

test("Enter saves journal entry to localStorage", async ({ nudge }) => {
    // Clear localStorage first
    await nudge.page.evaluate(() => localStorage.clear())

    // Type into fields
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 55)
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
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 55)
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

test("Multiple submits append to journal", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // First entry
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.type("first entry", { delay: 30 })
    await new Promise((r) => setTimeout(r, 200))
    await nudge.page.keyboard.press("Enter")
    await new Promise((r) => setTimeout(r, 500))

    // Nudge now to get form back
    await nudge.page.mouse.click(400, 120)
    await new Promise((r) => setTimeout(r, 500))

    // Second entry
    await nudge.page.mouse.click(400, 55)
    await new Promise((r) => setTimeout(r, 300))
    await nudge.page.mouse.click(400, 55)
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
