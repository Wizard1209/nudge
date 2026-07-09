import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import {
    visualAssert,
    PROMPT_CARD_PRESENT,
    PROMPT_CARD_FROSTED,
    PROMPT_THREE_ROWS,
    CARD_REGION,
} from "../fixtures/judge"

// All three asserts target the same fresh-load state and never mutate the
// popup, DOM, or localStorage — safe to share a single browser per file.
describe("static initial-load appearance (LLM-judged)", () => {
    test("fresh page load shows the input form card", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_CARD_PRESENT)
        console.log("card-present:", result)
        expect(result.pass).toBe(true)
    })

    test("card is a frosted floating surface", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_CARD_FROSTED)
        console.log("card-frosted:", result)
        expect(result.pass).toBe(true)
    })

    test("card shows three equal-height single-line rows", async ({ nudgeFile }) => {
        const screenshot = (await nudgeFile.page.screenshot()) as Buffer
        const result = await visualAssert(screenshot, PROMPT_THREE_ROWS, { region: CARD_REGION })
        console.log("three-rows:", result)
        expect(result.pass).toBe(true)
    })
})
