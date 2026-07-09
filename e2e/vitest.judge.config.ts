import { defineConfig } from "vitest/config"

// The LLM-judge group: only *.judge.test.ts files. Requires OPENAI_API_KEY and
// fails loudly without it — no silent skips. Run with `npm run test:judge`;
// in CI this group runs only in the release workflow, on top of the standard
// suite (vitest.config.ts, which excludes these files).
export default defineConfig({
    test: {
        include: ["tests/**/*.judge.test.ts"],
        testTimeout: 60_000,
        hookTimeout: 30_000,
        // Sequential files: stay under the judge model's TPM rate limit and
        // keep verdicts stable. Each test uses ~1k tokens.
        fileParallelism: false,
    },
})
