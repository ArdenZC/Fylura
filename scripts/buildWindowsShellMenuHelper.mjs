import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

if (process.platform !== "win32") {
  process.exit(0);
}

function run(command, args) {
  const isBatch = command.toLowerCase().endsWith(".cmd");
  const result = spawnSync(isBatch ? process.env.ComSpec ?? "cmd.exe" : command,
    isBatch ? ["/d", "/s", "/c", "call", command, ...args] : args,
    { cwd: root, env: process.env, stdio: "inherit", windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const release = process.argv.includes("--release");
const host = execFileSync("rustc", ["-vV"], { cwd: root, encoding: "utf8" })
  .match(/^host: (.+)$/m)?.[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE
  ?? process.env.CARGO_BUILD_TARGET
  ?? host;
if (!target?.startsWith("x86_64-pc-windows-")) {
  throw new Error(`The Windows Shell helper is qualified for x64 Windows; got ${target ?? "unknown target"}.`);
}

run("cargo", ["build", "--locked", "--manifest-path", "src-tauri/Cargo.toml", ...(release ? ["--release"] : []), "--bin", "fylura-shell-menu-helper"]);
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", "src-tauri/Cargo.toml"], { cwd: root, encoding: "utf8" }));
const profileDir = release ? "release" : "debug";
const executable = path.join(metadata.target_directory, profileDir, "fylura-shell-menu-helper.exe");
if (!fs.existsSync(executable)) throw new Error(`The Shell helper build output is missing: ${executable}`);

if (release) {
  const staged = path.join(root, "src-tauri", "binaries", `fylura-shell-menu-helper-${target}.exe`);
  fs.mkdirSync(path.dirname(staged), { recursive: true });
  fs.copyFileSync(executable, staged);
}
