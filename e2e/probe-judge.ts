// Probe script: captures canonical screenshots once, then hammers the LLM
// judge with prompts against those screenshots to verify both recall (positive
// claim on the right state always passes) and precision (positive claim on the
// wrong state always fails). Re-run this whenever you change a prompt or the
// underlying visuals to confirm the judge stays deterministic without retries.
//
// Usage:
//   cd e2e
//   npx tsx probe-judge.ts capture     # one-time: launch browser, save PNGs to /tmp/nudge-probe/
//   npx tsx probe-judge.ts probe       # run prompts against saved PNGs, print pass-rate table

import puppeteer from "puppeteer"
import fs from "node:fs"
import {
    visualAssert,
    CARD_REGION,
    BOTTOM_RIGHT_REGION,
    PROMPT_CARD_PRESENT,
    PROMPT_CARD_ABSENT,
    PROMPT_CARD_FROSTED,
    PROMPT_PILL_PRESENT,
    PROMPT_THREE_ROWS,
    promptTypedText,
    type Region,
} from "./fixtures/judge.ts"

const SCREENSHOT_DIR = "/tmp/nudge-probe"
const N_SAMPLES = 6
const PACE_MS = 1500

const STATES = ["initial", "pill-only", "typed"] as const
type State = typeof STATES[number]

// ────────────────────────────────────────────────────────────────────
// Capture
// ────────────────────────────────────────────────────────────────────

async function capture() {
    fs.mkdirSync(SCREENSHOT_DIR, { recursive: true })

    const browser = await puppeteer.launch({
        headless: false,
        args: ["--ignore-gpu-blocklist", "--no-sandbox", "--disable-setuid-sandbox", "--window-size=800,600"],
    })
    const page = await browser.newPage()
    await page.setViewport({ width: 800, height: 600 })
    await page.goto("http://localhost:8080", { waitUntil: "networkidle0" })
    await page.waitForSelector("canvas#nudge_canvas", { timeout: 5_000 })
    await new Promise((r) => setTimeout(r, 1_000))

    await page.screenshot({ path: `${SCREENSHOT_DIR}/initial.png` })
    console.log("captured: initial")

    await page.mouse.click(400, 170)
    await new Promise((r) => setTimeout(r, 300))
    await page.mouse.click(400, 170)
    await new Promise((r) => setTimeout(r, 200))
    await page.keyboard.type("hello nudge", { delay: 30 })
    await new Promise((r) => setTimeout(r, 400))
    await page.screenshot({ path: `${SCREENSHOT_DIR}/typed.png` })
    console.log("captured: typed")

    await page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 800))
    await page.screenshot({ path: `${SCREENSHOT_DIR}/pill-only.png` })
    console.log("captured: pill-only")

    await browser.close()
}

// ────────────────────────────────────────────────────────────────────
// Probe
// ────────────────────────────────────────────────────────────────────

interface PromptCase {
    state: State
    name: string
    expected: boolean
    prompt: string
    region?: Region
}

const CASES: PromptCase[] = [
    // Positives — must always pass
    { state: "initial",   name: "card-present/initial",   expected: true,  prompt: PROMPT_CARD_PRESENT },
    { state: "initial",   name: "card-frosted/initial",   expected: true,  prompt: PROMPT_CARD_FROSTED },
    { state: "initial",   name: "three-rows/initial",     expected: true,  prompt: PROMPT_THREE_ROWS,                region: CARD_REGION },
    { state: "pill-only", name: "card-absent/pill",       expected: true,  prompt: PROMPT_CARD_ABSENT },
    { state: "pill-only", name: "pill-present/pill",      expected: true,  prompt: PROMPT_PILL_PRESENT,              region: BOTTOM_RIGHT_REGION },
    { state: "typed",     name: "typed-hello/typed",      expected: true,  prompt: promptTypedText("hello nudge"),   region: CARD_REGION },
    { state: "typed",     name: "card-present/typed",     expected: true,  prompt: PROMPT_CARD_PRESENT },

    // Negatives — must always fail (precision check)
    { state: "pill-only", name: "NEG card-present/pill",  expected: false, prompt: PROMPT_CARD_PRESENT },
    { state: "initial",   name: "NEG card-absent/initial",expected: false, prompt: PROMPT_CARD_ABSENT },
    { state: "initial",   name: "NEG pill-present/init",  expected: false, prompt: PROMPT_PILL_PRESENT,              region: BOTTOM_RIGHT_REGION },
    { state: "pill-only", name: "NEG three-rows/pill",    expected: false, prompt: PROMPT_THREE_ROWS,                region: CARD_REGION },
    { state: "initial",   name: "NEG typed-hello/initial",expected: false, prompt: promptTypedText("hello nudge"),   region: CARD_REGION },
]

async function probe() {
    const bufs = new Map<State, Buffer>()
    for (const s of STATES) {
        bufs.set(s, fs.readFileSync(`${SCREENSHOT_DIR}/${s}.png`))
    }

    console.log(`\nProbing ${CASES.length} prompts × ${N_SAMPLES} samples\n`)
    console.log("name                          expected  pass-rate  worst-comment")
    console.log("─".repeat(95))

    for (const c of CASES) {
        const raw = bufs.get(c.state)!
        let passes = 0
        let firstUnexpected: string | null = null
        for (let i = 0; i < N_SAMPLES; i++) {
            try {
                const j = await visualAssert(raw, c.prompt, c.region ? { region: c.region } : {})
                if (j.pass) passes++
                if (j.pass !== c.expected && firstUnexpected === null) {
                    firstUnexpected = j.comment
                }
            } catch (e: any) {
                if (firstUnexpected === null) firstUnexpected = `ERR: ${(e?.message ?? String(e)).slice(0, 60)}`
            }
            await new Promise((r) => setTimeout(r, PACE_MS))
        }
        const ok = c.expected ? passes : N_SAMPLES - passes
        const rate = `${ok}/${N_SAMPLES}`
        const flag = ok === N_SAMPLES ? "✓" : ok === 0 ? "✗" : "~"
        console.log(
            `${flag} ${c.name.padEnd(28)} ${String(c.expected).padEnd(8)} ${rate.padEnd(10)} ${(firstUnexpected ?? "").slice(0, 60)}`
        )
    }
}

const cmd = process.argv[2]
if (cmd === "capture") {
    capture().catch((e) => { console.error(e); process.exit(1) })
} else if (cmd === "probe") {
    probe().catch((e) => { console.error(e); process.exit(1) })
} else {
    console.error("Usage: probe-judge.ts capture | probe")
    process.exit(1)
}
