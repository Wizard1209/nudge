import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { assertCardHidden } from "../fixtures/pixels"
import { wait } from "../fixtures/actions"

// Spec §4 switch-away rule must fire whenever the page is not focused,
// regardless of whether a `blur` event was actually dispatched. Some
// browser actions (closing a sibling tab, devtools stealing focus,
// renderer-side races) leave `document.hasFocus()` === false without
// firing window.blur. The popup must still close.
//
// We stub `document.hasFocus` so the test can drive it directly,
// WITHOUT dispatching any FocusEvent. Edge-triggered (listen-for-blur)
// implementations cannot pass this; only a per-frame poll of
// `document.hasFocus()` can.
describe("popup polls document.hasFocus()", () => {
    test("hides when hasFocus() becomes false without a blur event", async ({ nudge }) => {
        await nudge.page.evaluate(() => {
            let focused = true
            Object.defineProperty(document, "hasFocus", {
                configurable: true,
                value: () => focused,
            })
            ;(window as unknown as { __setHasFocus: (v: boolean) => void }).__setHasFocus = (v) => {
                focused = v
            }
        })

        // Engage the user so any user_engaged gate is satisfied — we are
        // testing the focus-polling path, not the gate.
        await nudge.page.mouse.click(400, 170)
        await wait(200)
        await nudge.page.keyboard.type("polling test", { delay: 30 })
        await wait(300)

        // Flip hasFocus → false. Crucially: no FocusEvent is dispatched.
        await nudge.page.evaluate(() =>
            (window as unknown as { __setHasFocus: (v: boolean) => void }).__setHasFocus(false))
        await wait(1500)

        await assertCardHidden(nudge.page)

        // Journal must still be empty — switch never writes (spec §4).
        const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        expect(journal ?? "").not.toContain("polling test")
    })
})
