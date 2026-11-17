import puppeteer, { type Browser, type Page } from "puppeteer"
import { test as base } from "vitest"

export interface NudgeContext {
    browser: Browser
    page: Page
}

const NUDGE_URL = "http://localhost:8080"

async function launchNudge(): Promise<NudgeContext> {
    const browser = await puppeteer.launch({
        headless: true,
        args: [
            "--ignore-gpu-blocklist",
            "--no-sandbox",
            "--disable-setuid-sandbox",
            "--window-size=800,600",
            "--force-device-scale-factor=1",
        ],
    })

    const page = await browser.newPage()
    await page.setViewport({ width: 800, height: 600 })
    await page.goto(NUDGE_URL, { waitUntil: "networkidle0" })

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
    nudgeFile: [
        async ({}, use) => {
            const ctx = await launchNudge()
            await use(ctx)
            await ctx.browser.close()
        },
        { scope: "file" },
    ],
})
