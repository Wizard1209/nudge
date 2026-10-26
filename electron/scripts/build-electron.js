/**
 * Compiles Electron main + preload TypeScript files to dist/electron/.
 * Uses esbuild for fast compilation (bundled with Vite).
 */
import { build } from "vite";
import { resolve } from "node:path";
import { writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";

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
          "node:module",
          "node:zlib",
          // A native binding: resolved from node_modules at run time.
          "@resvg/resvg-js",
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

  // The macOS glass under the card is a native addon (native/backdrop); it is
  // N-API, so one build serves node and Electron alike. Nothing to build
  // elsewhere: main.ts loads it on darwin only. A Mac where node-gyp cannot
  // build it (no Xcode command line tools, say) still gets a working popup
  // — main.ts runs without the glass and the card's fill carries the 95% —
  // so the build goes on; the native backdrop tests are what demand the addon.
  if (process.platform === "darwin") {
    try {
      buildBackdropAddon();
    } catch (err) {
      console.warn(
        `\nWARNING: the backdrop addon did not build (${err.message.trim()}).\n` +
          "The popup will run without the macOS glass under the card.\n" +
          "pnpm build:backdrop rebuilds it once node-gyp can; tests/native/backdrop.test.js fails until then.\n",
      );
    }
  }
}

export function buildBackdropAddon() {
  const nodeGyp = createRequire(import.meta.url).resolve("node-gyp/bin/node-gyp.js");
  execFileSync(process.execPath, [nodeGyp, "rebuild", "-C", resolve("native/backdrop")], {
    stdio: "inherit",
  });
  console.log("Backdrop addon built to native/backdrop/build/");
}

// Allow running directly
const isMain = process.argv[1]?.endsWith("build-electron.js");
if (isMain) {
  buildElectronFiles();
}
