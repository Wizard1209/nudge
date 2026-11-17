import { expect } from "vitest"
import { test } from "../fixtures/nudge"
import { decode, avgBrightness } from "../fixtures/pixels"
import {
    visualAssert,
    PROMPT_PILL_PRESENT,
    PROMPT_CARD_PRESENT,
    BOTTOM_RIGHT_REGION,
} from "../fixtures/judge"
import { dismissForm, clickPill, wait, PILL_X, PILL_Y } from "../fixtures/actions"

const PILL_SAMPLE_SIZE = 16

test("dismissed form shows a pill that highlights on hover and re-opens on click", async ({ nudge }) => {
    await dismissForm(nudge.page, "esc")
    const dismissedShot = (await nudge.page.screenshot()) as Buffer
    const pillJudge = await visualAssert(dismissedShot, PROMPT_PILL_PRESENT, { region: BOTTOM_RIGHT_REGION })
    console.log("pill shape:", pillJudge)
    expect(pillJudge.pass).toBe(true)

    await nudge.page.mouse.move(20, 20)
    await wait(400)
    const before = decode((await nudge.page.screenshot()) as Buffer)

    await nudge.page.mouse.move(PILL_X, PILL_Y)
    await wait(600)
    const after = decode((await nudge.page.screenshot()) as Buffer)

    const b0 = avgBrightness(before, PILL_X - PILL_SAMPLE_SIZE / 2, PILL_Y - PILL_SAMPLE_SIZE / 2, PILL_SAMPLE_SIZE, PILL_SAMPLE_SIZE)
    const b1 = avgBrightness(after, PILL_X - PILL_SAMPLE_SIZE / 2, PILL_Y - PILL_SAMPLE_SIZE / 2, PILL_SAMPLE_SIZE, PILL_SAMPLE_SIZE)
    console.log(`pill hover: before=${b0.toFixed(1)} after=${b1.toFixed(1)}`)
    expect(b1).toBeGreaterThan(b0 + 3)

    await clickPill(nudge.page)
    const reopenedShot = (await nudge.page.screenshot()) as Buffer
    const reopenJudge = await visualAssert(reopenedShot, PROMPT_CARD_PRESENT)
    console.log("pill re-open:", reopenJudge)
    expect(reopenJudge.pass).toBe(true)
})
