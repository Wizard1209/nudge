import { expect, describe } from "vitest"
import { test } from "../fixtures/nudge"
import { assertCardHidden } from "../fixtures/pixels"
import {
    focusField,
    selectAllAndType,
    wait,
    FIELD_DOING_Y,
    FIELD_MINUTES_Y,
} from "../fixtures/actions"

// Enter and Escape both hide the form — same dismiss contract via two keys.
describe("dismiss key hides the form", () => {
    test.for(["Enter", "Escape"] as const)("%s hides form and shows the pill", async (key, { nudge }) => {
        await nudge.page.keyboard.press(key)
        await wait(1000)
        await assertCardHidden(nudge.page)
    })
})

// "Timer auto-triggers form reappearance" is LLM-judged and lives in
// form-reopen.judge.test.ts; deterministic timer-reopen coverage is below in
// "Multiple submits append to journal (timer-driven reopen)".

test("Enter saves journal entry to localStorage", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    await focusField(nudge.page, FIELD_DOING_Y)
    await nudge.page.keyboard.type("writing code", { delay: 30 })
    await wait(200)

    // Tab to second field.
    await nudge.page.keyboard.press("Tab")
    await wait(200)
    await nudge.page.keyboard.type("no", { delay: 30 })
    await wait(200)

    await nudge.page.keyboard.press("Enter")
    await wait(500)

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
    expect(entry.trigger_source).toBe("timer")
    expect(entry.doing).toBe("writing code")
    expect(entry.bullshit).toBe("no")
    expect(entry.next_interval_minutes).toBe(10)
})

test("Esc does NOT save journal entry", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    await focusField(nudge.page, FIELD_DOING_Y)
    await nudge.page.keyboard.type("should not save", { delay: 30 })
    await wait(200)

    await nudge.page.keyboard.press("Escape")
    await wait(500)

    const journal = await nudge.page.evaluate(() => localStorage.getItem("journal"))
    console.log("Journal after Esc:", journal)
    expect(journal).toBeNull()
})

test("Multiple submits append to journal (timer-driven reopen)", async ({ nudge }) => {
    await nudge.page.evaluate(() => localStorage.clear())

    // First entry — set short timer so reopen fires within the test.
    await focusField(nudge.page, FIELD_MINUTES_Y)
    await selectAllAndType(nudge.page, "0.02")

    await focusField(nudge.page, FIELD_DOING_Y)
    await nudge.page.keyboard.type("first entry", { delay: 30 })
    await wait(200)
    await nudge.page.keyboard.press("Enter")

    // Wait for timer auto-reopen.
    await wait(2500)

    // Second entry.
    await focusField(nudge.page, FIELD_DOING_Y)
    await nudge.page.keyboard.type("second entry", { delay: 30 })
    await wait(200)
    await nudge.page.keyboard.press("Enter")
    await wait(500)

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
