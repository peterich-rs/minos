import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

// Write a tauri.release.conf.json with release-only overrides.
//
// Tauri's --config flag merges the provided JSON on top of the base
// tauri.conf.json, so this file must contain ONLY the delta fields —
// not a copy of the base config.
//
// Release builds emit:
// 1. bundle.createUpdaterArtifacts = true so Tauri produces the .tar.gz
//    (or platform archive) and .sig during the build.
// 2. plugins.updater with pubkey + endpoint from env vars.
//    Both MINOS_UPDATER_PUBLIC_KEY and MINOS_UPDATER_ENDPOINT are required.
//
// Usage (CI):
//   MINOS_UPDATER_PUBLIC_KEY=... \
//   MINOS_UPDATER_ENDPOINT=https://github.com/<org>/Minos/releases/download/minos-desktop-latest/latest.json \
//   node apps/desktop/scripts/build-release-config.mjs
//
// Then:
//   cargo tauri build --config src-tauri/tauri.release.conf.json
// with the same MINOS_UPDATER_* env vars so build.rs enables the plugin.

const outputConfigPath = resolve(
  process.cwd(),
  "src-tauri/tauri.release.conf.json",
);

const updaterPubkey = process.env.MINOS_UPDATER_PUBLIC_KEY;
const updaterEndpoint = process.env.MINOS_UPDATER_ENDPOINT;

const missing = [];
if (!updaterPubkey) missing.push("MINOS_UPDATER_PUBLIC_KEY");
if (!updaterEndpoint) missing.push("MINOS_UPDATER_ENDPOINT");
if (missing.length > 0) {
  console.error(
    `Error: required environment variable(s) missing: ${missing.join(", ")}`,
  );
  process.exit(1);
}

const releaseConfig = {
  bundle: {
    macOS: {
      minimumSystemVersion: "11.0",
    },
    createUpdaterArtifacts: true,
  },
  plugins: {
    updater: {
      pubkey: updaterPubkey,
      endpoints: [updaterEndpoint],
    },
  },
};

console.log(`Updater enabled -> ${updaterEndpoint}`);

writeFileSync(outputConfigPath, `${JSON.stringify(releaseConfig, null, 2)}\n`);
console.log(`Wrote ${outputConfigPath}`);
