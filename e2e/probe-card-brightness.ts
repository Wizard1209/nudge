// One-shot calibration: launch the app, capture both states, print the
// CARD_REGION brightness for each. Use the spread to pick assertCardHidden /
// assertCardVisible thresholds.
import puppeteer from "puppeteer"
import { decode, avgBrightness } from "./fixtures/pixels"
import { CARD_REGION } from "./fixtures/judge"

async function main() {
    const browser = await puppeteer.launch({
        headless: false,
        args: ["--ignore-gpu-blocklist", "--no-sandbox", "--disable-setuid-sandbox", "--window-size=800,600"],
    })
    const page = await browser.newPage()
    await page.setViewport({ width: 800, height: 600 })
    await page.goto("http://localhost:8080", { waitUntil: "networkidle0" })
    await page.waitForSelector("canvas#nudge_canvas", { timeout: 5_000 })
    await new Promise((r) => setTimeout(r, 1_000))

    const visible = decode(await page.screenshot() as Buffer)
    const vB = avgBrightness(visible, CARD_REGION.x, CARD_REGION.y, CARD_REGION.w, CARD_REGION.h)
    console.log(`card VISIBLE: avgBrightness(CARD_REGION) = ${vB.toFixed(2)}`)

    await page.keyboard.press("Escape")
    await new Promise((r) => setTimeout(r, 800))

    const hidden = decode(await page.screenshot() as Buffer)
    const hB = avgBrightness(hidden, CARD_REGION.x, CARD_REGION.y, CARD_REGION.w, CARD_REGION.h)
    console.log(`card HIDDEN : avgBrightness(CARD_REGION) = ${hB.toFixed(2)}`)

    console.log(`spread = ${(hB - vB).toFixed(2)}`)

    await browser.close()
}

main().catch((e) => { console.error(e); process.exit(1) })
