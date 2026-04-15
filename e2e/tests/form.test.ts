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

test("can type into the first field", async ({ nudge }) => {
    // Click directly on the first text field (white rectangle below "Что я делаю?")
    // From screenshot: label at y~5-15, field at y~18-38
    await nudge.page.mouse.click(150, 28)
    await new Promise((r) => setTimeout(r, 500))

    // egui needs a second click sometimes to activate text edit
    await nudge.page.mouse.click(150, 28)
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
