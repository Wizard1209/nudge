import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { assertCardHidden } from "../fixtures/pixels"
import { wait } from "../fixtures/actions"

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

        await wait(300)

        await nudge.page.evaluate(() =>
            (window as unknown as { __setHasFocus: (v: boolean) => void }).__setHasFocus(false))
        await wait(1500)

        await assertCardHidden(nudge.page)

        const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
        expect(journal ?? "").not.toContain("polling test")
    })
})
