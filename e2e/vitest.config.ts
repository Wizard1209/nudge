import { defineConfig, configDefaults } from "vitest/config"

export default defineConfig({
    test: {
        testTimeout: 60_000,
        hookTimeout: 30_000,
        // Run test files sequentially to keep a single browser's worth of load
        // and stable timings.
        fileParallelism: false,
        // The standard suite is fully deterministic. LLM-judged tests live in
        // *.judge.test.ts and run as a separate group (vitest.judge.config.ts)
        // that requires OPENAI_API_KEY — in CI it runs only for releases, on
        // top of this suite.
        exclude: [...configDefaults.exclude, "**/*.judge.test.ts"],
    },
})
