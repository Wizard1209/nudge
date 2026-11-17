import http from 'node:http';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import puppeteer from 'puppeteer';

const BASE_URL = process.env.E2E_BASE_URL || 'http://localhost:5173';

function missingBrowser() {
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
