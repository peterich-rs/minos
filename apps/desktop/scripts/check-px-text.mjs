#!/usr/bin/env node
import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { runPxTextCheck } from "./check-px-text-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");
const allowlistPath = path.join(__dirname, "check-px-text.allowlist.txt");

const rules = [
  {
    root: "src",
    extensions: new Set([".ts", ".tsx", ".css"]),
  },
];

const allowlistText = await fs.readFile(allowlistPath, "utf8");
const overrides = new Set(
  allowlistText
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith("#")),
);

await runPxTextCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "apps/desktop/scripts/check-px-text.allowlist.txt",
});
