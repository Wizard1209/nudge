import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { assertCardHidden } from "../fixtures/pixels"
import { clickPill, dismissForm, wait, type DismissTrigger } from "../fixtures/actions"

// Esc, blur and click-outside all follow the same contract per spec §4:
// the popup hides without recording an entry AND the typed text survives
// until the user reopens via the pill and presses Enter. Esc shares its
// row with Switch in the spec table — different trigger, same effect.
//
// One parametrized scenario over the three triggers, so the contract stays
// authoritative in one place. The trigger names are also the test-case
// labels so a failure prints which path broke.
const TRIGGERS: ReadonlyArray<[label: string, via: DismissTrigger, text: string]> = [
    ["esc",           "esc",           "preserved across esc"],
    ["click-outside", "click-outside", "preserved across switch"],
    ["blur",          "blur",          "preserved across blur"],
]

describe("popup preserves typed text across switch-away", () => {
    test.for(TRIGGERS)("%s trigger hides popup, keeps text, restores on reopen", async ([_, via, text], { nudge }) => {
        await nudge.page.evaluate(() => localStorage.clear())

        // Type into the first (auto-focused) field.
        await nudge.page.mouse.click(400, 170)
        await wait(200)
        await nudge.page.keyboard.type(text, { delay: 30 })
        await wait(300)

        // Trigger the switch-away.
        await dismissForm(nudge.page, via)

        // Card must be hidden (pixel check — deterministic, no LLM).
        await assertCardHidden(nudge.page)

        // Journal must still be empty — switch never writes.
        let journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        expect(journal ?? "").not.toContain(text)

        // Re-open via pill, then Enter — typed text should land in the journal.
        await clickPill(nudge.page)
        await nudge.page.keyboard.press("Enter")
        await wait(500)

        journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        console.log(`journal after reopen via ${via}:`, journal)
        expect(journal ?? "").toContain(text)
    })
})
