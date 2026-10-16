import http from 'node:http';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import puppeteer from 'puppeteer';

const BASE_URL = process.env.E2E_BASE_URL || 'http://localhost:5173';

/**
 * Puppeteer downloads its browser in a postinstall script, and pnpm skips that
 * script whenever it can restore the package from its side-effects cache — a
 * restore that cannot bring back a download living outside node_modules. So an
 * install that reports success is no proof the browser is here; say which
 * command puts it back, rather than letting ChromeLauncher fail cryptically.
 */
function missingBrowser() {
    // The suite launches chrome-headless-shell, and headed Chrome under
    // E2E_DEBUG=1 (fixtures/browser.js). Look in the cache directory rather
    // than asking for an executable path: `executablePath` does not take
    // chrome-headless-shell as a product name.
    const needed = process.env.E2E_DEBUG ? 'chrome' : 'chrome-headless-shell';
    const cacheDir = puppeteer.configuration?.cacheDirectory;
    if (!cacheDir) return null;
    return existsSync(join(cacheDir, needed)) ? null : needed;
}

function checkServer(url) {
    return new Promise((resolve) => {
        http.get(url, (res) => {
            resolve(res.statusCode < 500);
        }).on('error', () => resolve(false));
    });
}

export default async function setup() {
    const isUp = await checkServer(BASE_URL);
    if (!isUp) {
        throw new Error(
            `Server not reachable at ${BASE_URL}.\n`
            + 'Run "pnpm dev" first, or set E2E_BASE_URL.',
        );
    }
    const missing = missingBrowser();
    if (missing) {
        throw new Error(
            `Puppeteer has no ${missing} installed.\n`
            + `Run "npx puppeteer browsers install ${missing}".`,
        );
    }
}
