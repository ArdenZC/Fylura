import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { classifyCiScope } from "./classifyCiChanges.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function parseOptions(args) {
  const result = { planOnly: false, full: false, rustTest: "", base: "" };
  for (const arg of args) {
    if (arg === "--plan") result.planOnly = true;
    else if (arg === "--full") result.full = true;
    else if (arg.startsWith("--base=") && arg.length > 7) result.base = arg.slice(7);
    else if (arg.startsWith("--rust-test=") && arg.length > 12) result.rustTest = arg.slice(12);
    else throw new Error("Unknown/empty option: " + arg + ". Use --plan, --full, --base=REF, --rust-test=FILTER.");
  }
  if (result.full && result.rustTest) throw new Error("Use either --full or --rust-test, not both.");
  return result;
}
function git(args) {
  return execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();
}
function gitPathList(args) {
  return execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).split("\0").filter(Boolean);
}
export function changedPathsForPreflight(baseRef = "") {
  const base = baseRef || ["origin/master", "master"].find((ref) => {
    try { git(["rev-parse", "--verify", ref + "^{commit}"]); return true; }
    catch { return false; }
  });
  if (!base) throw new Error("Cannot find origin/master or master. Supply --base=REF (an existing local git ref).");
  const mergeBase = git(["merge-base", base, "HEAD"]);
  if (!mergeBase) throw new Error("No merge base with " + base);
  return [...new Set([
    ...gitPathList(["diff", "--name-only", "-z", mergeBase, "HEAD"]),
    ...gitPathList(["diff", "--name-only", "-z", "HEAD"]),
    ...gitPathList(["ls-files", "--others", "--exclude-standard", "-z"]),
  ])];
}
export function buildPreflightPlan(paths, options = {}) {
  const changes = [...new Set(paths)];
  const scope = classifyCiScope({ event: "pull_request", changedPaths: changes });
  const commands = [];
  const add = (label, command, args) => commands.push({ label, command, args });
  if (changes.length === 0) return { scope, commands, warnings: ["No changed files; no checks selected."] };
  if (scope.docs_only) add("Documentation contract", "npm", ["run", "test:docs"]);
  else {
    if (scope.frontend_changed) {
      add("TypeScript", "npm", ["run", "typecheck"]);
      add("Frontend tests", "npm", ["test"]);
      add("Frontend build", "npm", ["run", "build:frontend"]);
    }
    if (scope.rust_changed) {
      add("Rust formatting", "cargo", ["fmt", "--manifest-path", "src-tauri/Cargo.toml", "--", "--check"]);
      add("Rust Clippy", "cargo", ["clippy", "--manifest-path", "src-tauri/Cargo.toml", "--features", "desktop-runtime", "--all-targets", "--", "-D", "warnings"]);
      if (options.full) add("Rust regression suite", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--features", "desktop-runtime"]);
      else if (options.rustTest) add("Focused Rust regression", "cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--features", "desktop-runtime", "--lib", options.rustTest]);
    }
    if (changes.some((item) => item.replaceAll("\\", "/").toLowerCase().startsWith("src-tauri/native/"))) {
      add("Native Rust tests", "cargo", ["test", "--manifest-path", "src-tauri/native/Cargo.toml"]);
    }
    if (changes.some((item) => item.startsWith("scripts/") || item.startsWith(".github/workflows/"))) {
      add("Governance scripts", "npm", ["run", "test:governance"]);
    }
  }
  const warnings = [];
  if (scope.rust_changed && !options.full && !options.rustTest) warnings.push("Rust runtime tests not run in quick mode; add --rust-test=FILTER for affected behavior, or --full for the suite.");
  if (scope.high_risk) warnings.push("High-risk path: this helper does not replace required CI, native checks or Owner safety review.");
  if (!commands.length) warnings.push("No local check mapped to these paths. Review the applicable existing CI requirements.");
  return { scope, commands, warnings };
}
export function runPreflight(args = process.argv.slice(2)) {
  const options = parseOptions(args);
  const paths = changedPathsForPreflight(options.base);
  const plan = buildPreflightPlan(paths, options);
  console.log("Changed files: " + paths.length + "; selected local checks: " + plan.commands.length);
  plan.commands.forEach((item, index) => console.log((index + 1) + ". " + item.label + ": " + item.command + " " + item.args.join(" ")));
  plan.warnings.forEach((message) => console.log("NOTE: " + message));
  if (options.planOnly) return 0;
  for (const item of plan.commands) {
    console.log("\n[preflight] " + item.label);
    const command = process.platform === "win32" && item.command === "npm" ? "npm.cmd" : item.command;
    const result = spawnSync(command, item.args, { cwd: ROOT, stdio: "inherit", shell: process.platform === "win32" && item.command === "npm" });
    if (result.error) { console.error(result.error.message); return 1; }
    if (result.status !== 0) { console.error("[preflight] Failed: " + item.label); return result.status || 1; }
  }
  return 0;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try { process.exitCode = runPreflight(); }
  catch (error) { console.error("[preflight] " + error.message); process.exitCode = 1; }
}
