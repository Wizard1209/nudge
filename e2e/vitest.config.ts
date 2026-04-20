import { defineConfig } from "vitest/config"

export default defineConfig({
    test: {
        testTimeout: 60_000,
        hookTimeout: 30_000,
        // Run test files sequentially to stay under OpenAI's gpt-4o-mini TPM limit (200k)
        // and to keep LLM-judge stable across tests. Each test uses ~1k tokens.
        fileParallelism: false,
    },
})
