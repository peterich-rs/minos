#!/usr/bin/env node
/**
 * Tauri beforeDevCommand helper.
 *
 * "Share one port" means: one Vite process, many clients (browser + WebView).
 * It does NOT mean two Vite processes on :1420.
 *
 * If :1420 already serves the app (e.g. `just dev-desktop-ui`), reuse it and
 * idle so Tauri can attach. Otherwise start Vite.
 */
import { spawn } from "node:child_process";
import { createConnection } from "node:net";

const HOST = "127.0.0.1";
const PORT = Number(process.env.MINOS_DESKTOP_DEV_PORT || 1420);
const DEV_URL = `http://${HOST}:${PORT}`;

function portOpen(host, port) {
  return new Promise((resolve) => {
    const socket = createConnection({ host, port }, () => {
      socket.end();
      resolve(true);
    });
    socket.setTimeout(400);
    socket.on("timeout", () => {
      socket.destroy();
      resolve(false);
    });
    socket.on("error", () => resolve(false));
  });
}

async function main() {
  if (await portOpen(HOST, PORT)) {
    console.log(
      `[minos-desktop] Reusing existing dev server at ${DEV_URL} (browser + Tauri share this Vite).`,
    );
    console.log(
      "[minos-desktop] UI HMR applies to all clients connected to this port.",
    );
    // Stay alive: Tauri treats a non-zero/early exit of beforeDevCommand as failure.
    await new Promise(() => {});
    return;
  }

  console.log(`[minos-desktop] Starting Vite on ${DEV_URL}`);
  const child = spawn("pnpm", ["exec", "vite"], {
    stdio: "inherit",
    shell: process.platform === "win32",
    env: process.env,
  });

  const shutdown = (signal) => {
    if (!child.killed) child.kill(signal);
  };
  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));

  child.on("exit", (code, signal) => {
    if (signal) process.kill(process.pid, signal);
    process.exit(code ?? 1);
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
