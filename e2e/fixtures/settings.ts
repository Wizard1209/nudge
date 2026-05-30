import puppeteer, { type Browser, type Page } from "puppeteer"
import { test as base } from "vitest"

/**
 * Settings window fixture — boots `settings.html` instead of `index.html`.
 * The shared `wasm_entry::start` in lib.rs picks the SettingsApp branch
 * when the URL path or query contains "settings".
 *
 * Separate from the popup fixture so the two test suites don't accidentally
 * share a browser tab carrying stale URL state. Per the existing fixture
 * pattern (see fixtures/nudge.ts), each test gets a fresh browser since the
 * tests mutate localStorage.
 */

export interface SettingsContext {
    browser: Browser
    page: Page
}

const SETTINGS_URL = "http://localhost:8080/settings.html"

async function launchSettings(): Promise<SettingsContext> {
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
    await page.goto(SETTINGS_URL, { waitUntil: "networkidle0" })

    await page.waitForSelector("canvas#nudge_canvas", { timeout: 10_000 })
    // Same cold-paint allowance as the popup fixture — first WASM compile is
    // slow on a fresh browser.
    await new Promise((r) => setTimeout(r, 2_500))

    return { browser, page }
}

export const test = base.extend<{ settings: SettingsContext }>({
    settings: [
        async ({}, use) => {
            const ctx = await launchSettings()
            await use(ctx)
            await ctx.browser.close()
        },
        { scope: "test" },
    ],
})
