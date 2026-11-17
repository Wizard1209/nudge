import { spawn } from "node:child_process";
import { createServer } from "vite";
import { buildElectronFiles } from "./build-electron.js";

async function main() {
  await buildElectronFiles();

  const server = await createServer();
  await server.listen(5173);
  console.log("Vite dev server running on http://localhost:5173");

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
