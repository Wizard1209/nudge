import { defineConfig } from 'vitest/config';

export default defineConfig({
    test: {
        include: ['tests/e2e/**/*.test.js'],
        environment: 'node',
        globalSetup: './tests/e2e/global-setup.js',
        testTimeout: 30_000,
        hookTimeout: 60_000,
        fileParallelism: false,
    },
});
