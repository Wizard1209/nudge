import puppeteer, { type Browser, type Page } from "puppeteer"
import { test as base } from "vitest"

export interface NudgeContext {
    browser: Browser
    page: Page
}

const NUDGE_URL = "http://localhost:8080"

async function launchNudge(): Promise<NudgeContext> {
    const browser = await puppeteer.launch({
        headless: false,
        args: [
            "--ignore-gpu-blocklist",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--window-size=800,600",
        ],
    })

    const page = await browser.newPage()
    await page.setViewport({ width: 800, height: 600 })
    await page.goto(NUDGE_URL, { waitUntil: "networkidle0" })

    // Wait for egui canvas to render (give it a moment to paint). The egui
    // 0.34 wasm bundle is ~2× larger (Vello), so first-load compile in a cold
    // browser is slower; 2.5s keeps the first test of a run from racing the
    // initial paint/focus. Subsequent loads are cached and fast.
    await page.waitForSelector("canvas#nudge_canvas", { timeout: 10_000 })
    await new Promise((r) => setTimeout(r, 2_500))

    return { browser, page }
}

export const test = base.extend<{ nudge: NudgeContext; nudgeFile: NudgeContext }>({
    nudge: [
        async ({}, use) => {
            const ctx = await launchNudge()
            await use(ctx)
            await ctx.browser.close()
        },
        { scope: "test" },
    ],
    // Shared per-file browser. Use this ONLY for tests that observe a static
    // initial state without mutating DOM/localStorage/popup state — pixel or
    // judge asserts on the fresh-load screenshot. Anything that types, clicks
    // into fields, dismisses the popup, or writes localStorage MUST use
    // `nudge` so it gets a fresh browser per test.
    nudgeFile: [
        async ({}, use) => {
            const ctx = await launchNudge()
            await use(ctx)
            await ctx.browser.close()
        },
        { scope: "file" },
    ],
})
