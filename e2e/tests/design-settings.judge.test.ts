import { expect, describe } from "vitest"
import { test } from "../fixtures/settings"
import {
    visualAssert,
    PROMPT_SETTINGS_HOTKEY_ROW,
    PROMPT_SETTINGS_FIELDS,
    PROMPT_SETTINGS_ARROWS_ARE_WORDS,
    SETTINGS_REGION,
} from "../fixtures/judge"
import { wait } from "../fixtures/actions"

describe("settings page copy (LLM-judged)", () => {
    async function shot(page: import("puppeteer").Page): Promise<Buffer> {
        await wait(700)
        return (await page.screenshot()) as Buffer
    }

    test("the hotkey row shows the platform spelling with the vocabulary under it (§9)", async ({ settings }) => {
        const result = await visualAssert(await shot(settings.page), PROMPT_SETTINGS_HOTKEY_ROW, { region: SETTINGS_REGION })
        console.log("hotkey row:", result)
        expect(result.pass).toBe(true)
    })

    test("the interval field and the launch checkbox carry their §9 labels", async ({ settings }) => {
        const result = await visualAssert(await shot(settings.page), PROMPT_SETTINGS_FIELDS, { region: SETTINGS_REGION })
        console.log("fields:", result)
        expect(result.pass).toBe(true)
    })

    test("the vocabulary spells the arrow keys as words this build can draw", async ({ settings }) => {
        const result = await visualAssert(await shot(settings.page), PROMPT_SETTINGS_ARROWS_ARE_WORDS, { region: SETTINGS_REGION })
        console.log("arrows:", result)
        expect(result.pass).toBe(true)
    })
})
