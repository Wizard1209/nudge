import http from 'node:http';

const BASE_URL = process.env.E2E_BASE_URL || 'http://localhost:5173';

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
}
