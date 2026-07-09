import puppeteer from 'puppeteer';

const BASE_URL = process.env.E2E_BASE_URL || 'http://localhost:5173';

// Debug mode: set E2E_DEBUG=1 to run headed with step delays and element highlights.
const DEBUG_MODE = !!process.env.E2E_DEBUG;
const STEP_DELAY = DEBUG_MODE ? 1500 : 0;

export async function step(ms = STEP_DELAY) {
    if (ms > 0) await new Promise((r) => { setTimeout(r, ms); });
}

/**
 * Highlight an element for visual debugging. No-op in headless.
 */
export async function highlight(page, selector, ms = STEP_DELAY) {
    if (!DEBUG_MODE) return;
    await page.evaluate((sel) => {
        const el = document.querySelector(sel);
        if (el) {
            el.style.outline = '3px solid #ff5252';
            el.style.outlineOffset = '-2px';
            el.style.transition = 'outline 0.2s';
        }
    }, selector);
    await step(ms);
    await page.evaluate((sel) => {
        const el = document.querySelector(sel);
        if (el) el.style.outline = '';
    }, selector);
}

/**
 * Launch a new browser instance.
 * In debug mode, launches headed (visible window) with slowMo.
 */
export async function launchBrowser() {
    return puppeteer.launch({
        headless: DEBUG_MODE ? false : 'shell',
        slowMo: DEBUG_MODE ? 50 : 0,
        args: [
            '--no-sandbox',
            '--disable-setuid-sandbox',
            '--window-size=800,600',
        ],
    });
}

/**
 * Open a page with error collection.
 */
export async function createPage(browser, path = '/') {
    const page = await browser.newPage();
    await page.setViewport({ width: 800, height: 600 });

    const consoleErrors = [];
    const pageErrors = [];

    page.on('console', (msg) => {
        if (msg.type() === 'error') consoleErrors.push(msg.text());
    });
    page.on('pageerror', (err) => pageErrors.push(err));

    await page.goto(`${BASE_URL}${path}`, { waitUntil: 'networkidle0' });
    await page.waitForSelector('#app', { timeout: 10_000 });
    await step();
    return { page, consoleErrors, pageErrors };
}
