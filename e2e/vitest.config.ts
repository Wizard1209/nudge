import { defineConfig, configDefaults } from "vitest/config"

export default defineConfig({
    test: {
        testTimeout: 60_000,
        hookTimeout: 30_000,
        fileParallelism: false,
        exclude: [...configDefaults.exclude, "**/*.judge.test.ts"],
    },
})
