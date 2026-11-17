import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import {
    visualAssert,
    PROMPT_PLACEHOLDERS,
    PROMPT_ROUNDED_CORNERS,
    PROMPT_SUBMIT_ERROR,
    promptTypedText,
    CARD_REGION,
    CARD_ERROR_REGION,
} from "../fixtures/judge"
import { focusField, selectAllAndType, wait, FIELD_DOING_Y, FIELD_MINUTES_Y } from "../fixtures/actions"

describe("popup copy and shape (LLM-judged)", () => {
    test("fresh load shows the three placeholders of spec §3", async ({ nudgeFile }) => {
        await wait(700)
        const shot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(shot, PROMPT_PLACEHOLDERS, { region: CARD_REGION })
        console.log("placeholders:", result)
        expect(result.pass).toBe(true)
    })

    test("the card's four corners are rounded (spec §2)", async ({ nudgeFile }) => {
        await wait(700)
        const shot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(shot, PROMPT_ROUNDED_CORNERS, { region: CARD_REGION })
        console.log("rounded corners:", result)
        expect(result.pass).toBe(true)
    })
})

test("typing replaces the placeholder with the typed text (spec §3)", async ({ nudge }) => {
    await focusField(nudge.page, FIELD_DOING_Y)
    await selectAllAndType(nudge.page, "writing tests")
    await wait(500)
    const shot = (await nudge.page.screenshot()) as Buffer
    const result = await visualAssert(shot, promptTypedText("writing tests"), { region: CARD_REGION })
    console.log("typed text:", result)
    expect(result.pass).toBe(true)
})

test("an invalid interval keeps the popup open and shows the error (spec §4)", async ({ nudge }) => {
    await focusField(nudge.page, FIELD_MINUTES_Y)
    await selectAllAndType(nudge.page, "-5")
    await nudge.page.keyboard.press("Enter")
    await wait(700)
    const shot = (await nudge.page.screenshot()) as Buffer
    const result = await visualAssert(shot, PROMPT_SUBMIT_ERROR, { region: CARD_ERROR_REGION })
    console.log("submit error:", result)
    expect(result.pass).toBe(true)
})
