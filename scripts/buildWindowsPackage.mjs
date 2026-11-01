import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  cleanWindowsNsisTemplate,
  prepareWindowsNsisTemplate,
} from "./prepareWindowsNsisLifecycleTemplate.mjs";
import { assertGeneratedPreviewResourcePathFile } from "./verifyWindowsNsisPreviewResource.mjs";
import { resolveTauriBuildFeatures } from "./tauriBuildFeatures.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const nativeBuildScript = path.join(repositoryRoot, "scripts", "buildWindowsPreviewHandler.mjs");
const packageConfig = path.join(
  repositoryRoot,
  "src-tauri",
  "tauri.windows.package.conf.json",
);
const packageResourcePath = path.join(
  repositoryRoot,
  "src-tauri",
  "native",
  "packaged",
  "fylura_windows_preview_handler.dll",
);

function run(command, args) {
  const isWindowsBatch = process.platform === "win32" && command.toLowerCase().endsWith(".cmd");
  const executable = isWindowsBatch ? process.env.ComSpec ?? "cmd.exe" : command;
  const commandArgs = isWindowsBatch
    ? ["/d", "/s", "/c", "call", command, ...args]
    : args;
  const result = spawnSync(executable, commandArgs, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error) {
    throw new Error(`Unable to start ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

let generatedNsisTemplate = false;
try {
  if (process.platform === "win32") {
    // Package-only inputs are produced immediately before Tauri bundles. The
    // composed NSIS generator proves the vendored source is the exact Tauri
    // CLI 2.11.2 blob, applies reviewed lifecycle call-site changes, and then
    // routes generated metadata/delete failures to the same synchronous owner.
    run(process.execPath, [nativeBuildScript]);
    if (!fs.existsSync(packageResourcePath)) {
      throw new Error(`Expected package staging DLL is missing: ${packageResourcePath}`);
    }
    prepareWindowsNsisTemplate();
    generatedNsisTemplate = true;
  }

  const tauriCommand = path.join(
    repositoryRoot,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "tauri.cmd" : "tauri",
  );
  const tauriFeatures = resolveTauriBuildFeatures();
  console.log(`Building Tauri package with Cargo features: ${tauriFeatures.join(",")}`);
  const tauriArgs = ["build", "--features", tauriFeatures.join(",")];
  if (process.platform === "win32") {
    tauriArgs.push("--config", packageConfig);
  }
  tauriArgs.push(...process.argv.slice(2));
  run(tauriCommand, tauriArgs);
  if (process.platform === "win32") {
    // Tauri has materialized the package-specific installer script now. Keep
    // the generated resource invocation accountable to the canonical NSIS
    // backslash destination and to the servicing macro's actual File command.
    assertGeneratedPreviewResourcePathFile();
  }
} finally {
  if (generatedNsisTemplate) {
    cleanWindowsNsisTemplate();
  }
}

