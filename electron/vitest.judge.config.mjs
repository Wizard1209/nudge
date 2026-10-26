import { defineConfig } from 'vitest/config';

// LLM-judge group: tests/e2e/*.judge.test.js. Needs OPENAI_API_KEY (env or
// repo-root .env) and a running dev server, like the e2e tier.
export default defineConfig({
    test: {
        include: ['tests/e2e/**/*.judge.test.js'],
        environment: 'node',
        globalSetup: './tests/e2e/global-setup.js',
        testTimeout: 90_000,
        hookTimeout: 60_000,
        fileParallelism: false,
    },
});
