#!/usr/bin/env node
/**
 * Verify a Tauri updater archive + .sig against MINOS_UPDATER_PUBLIC_KEY
 * using the same algorithm as tauri-plugin-updater (base64-decode sig → minisign).
 *
 * Usage: node scripts/verify-updater-sig.mjs <archive> <pubkey-base64>
 *
 * Spawns a tiny Rust program via `cargo script` is heavy; instead shell out to
 * a pre-defined inline rustc... We use the workspace `minos-desktop` helper bin.
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const archive = process.argv[2];
const pubkey = process.argv[3] ?? process.env.MINOS_UPDATER_PUBLIC_KEY;

if (!archive || !pubkey) {
  console.error(
    "usage: node scripts/verify-updater-sig.mjs <archive> <pubkey-base64>",
  );
  process.exit(2);
}

const archivePath = resolve(archive);
const sigPath = `${archivePath}.sig`;
if (!existsSync(archivePath) || !existsSync(sigPath)) {
  console.error(`missing archive or .sig: ${archivePath}`);
  process.exit(1);
}

const manifest = join(__dirname, "../src-tauri/Cargo.toml");
const result = spawnSync(
  "cargo",
  [
    "run",
    "--quiet",
    "--manifest-path",
    manifest,
    "--bin",
    "verify-updater-sig",
    "--",
    archivePath,
    pubkey.trim(),
  ],
  {
    stdio: "inherit",
    env: process.env,
  },
);

process.exit(result.status ?? 1);
