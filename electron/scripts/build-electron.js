/**
 * Compiles Electron main + preload TypeScript files to dist/electron/.
 * Uses esbuild for fast compilation (bundled with Vite).
 */
import { build } from "vite";
import { resolve } from "node:path";
import { writeFileSync, mkdirSync } from "node:fs";

export async function buildElectronFiles() {
  // Build main process
  await build({
    configFile: false,
    build: {
      outDir: "dist/electron",
      lib: {
        entry: resolve("src/electron/main.ts"),
        formats: ["cjs"],
        fileName: () => "main.js",
      },
      rollupOptions: {
        external: [
          "electron",
          "node:path",
          "node:fs",
          "node:url",
          "node:child_process",
          "node:zlib",
        ],
      },
      emptyOutDir: false,
      minify: false,
    },
  });

  // Build preload script
  await build({
    configFile: false,
    build: {
      outDir: "dist/electron",
      lib: {
        entry: resolve("src/electron/preload.ts"),
        formats: ["cjs"],
        fileName: () => "preload.js",
      },
      rollupOptions: {
        external: ["electron"],
      },
      emptyOutDir: false,
      minify: false,
    },
  });

  // Override parent package.json "type": "module" for this dir — emitted files are CJS
  mkdirSync(resolve("dist/electron"), { recursive: true });
  writeFileSync(
    resolve("dist/electron/package.json"),
    JSON.stringify({ type: "commonjs" }) + "\n",
    "utf-8",
  );

  console.log("Electron files compiled to dist/electron/");
}

// Allow running directly
const isMain = process.argv[1]?.endsWith("build-electron.js");
if (isMain) {
  buildElectronFiles();
}
