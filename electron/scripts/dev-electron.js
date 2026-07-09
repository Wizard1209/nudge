/**
 * Dev script: starts Vite dev server, then launches Electron pointing at it.
 * Electron main/preload are compiled with tsc on the fly.
 */
import { spawn } from "node:child_process";
import { createServer } from "vite";
import { buildElectronFiles } from "./build-electron.js";

async function main() {
  // 1. Compile Electron main + preload to dist/electron/
  await buildElectronFiles();

  // 2. Start Vite dev server
  const server = await createServer();
  await server.listen(5173);
  console.log("Vite dev server running on http://localhost:5173");

  // 3. Launch Electron
  const electronPath = (await import("electron")).default;
  const electronProcess = spawn(String(electronPath), ["dist/electron/main.js"], {
    stdio: "inherit",
    env: { ...process.env, NODE_ENV: "development" },
  });

  electronProcess.on("close", () => {
    server.close();
    process.exit(0);
  });
}

main();
