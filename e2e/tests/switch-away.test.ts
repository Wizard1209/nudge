import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { assertCardHidden } from "../fixtures/pixels"
import { clickPill, dismissForm, focusField, wait, FIELD_DOING_Y, type DismissTrigger } from "../fixtures/actions"

const TRIGGERS: ReadonlyArray<[label: string, via: DismissTrigger, text: string]> = [
    ["esc",           "esc",           "preserved across esc"],
    ["click-outside", "click-outside", "preserved across switch"],
    ["blur",          "blur",          "preserved across blur"],
]

describe("popup preserves typed text across switch-away", () => {
    test.for(TRIGGERS)("%s trigger hides popup, keeps text, restores on reopen", async ([_, via, text], { nudge }) => {
        await nudge.page.evaluate(() => localStorage.clear())

        await focusField(nudge.page, FIELD_DOING_Y)
        await nudge.page.keyboard.type(text, { delay: 30 })
        await wait(300)

        await dismissForm(nudge.page, via)

        await assertCardHidden(nudge.page)

        let journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        expect(journal ?? "").not.toContain(text)

        await clickPill(nudge.page)
        await nudge.page.keyboard.press("Enter")
        await wait(500)

        journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        console.log(`journal after reopen via ${via}:`, journal)
        expect(journal ?? "").toContain(text)
    })
})
