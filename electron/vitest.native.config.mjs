import { defineConfig } from 'vitest/config';

// Native Electron integration tests. Unlike the Puppeteer e2e suite these
// launch the real app (no Vite dev server — they load the built renderer via
// NUDGE_FORCE_PROD), so `pnpm build && pnpm build:electron` must run first.
//
// Requirements: a display (WSLg/X11) and an UNSANDBOXED process (Electron must
// open the X socket). See tests/native/fixtures/electron.js.
export default defineConfig({
  test: {
    include: ['tests/native/**/*.test.js'],
    environment: 'node',
    testTimeout: 60_000,
    hookTimeout: 60_000,
    fileParallelism: false,
  },
});
