import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { buildFyluraNsisTemplate } from "../scripts/prepareWindowsNsisLifecycleTemplate.mjs";

const repositoryRoot = path.resolve(import.meta.dirname, "..");
const upstreamPath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "tauri-2.11.2-installer.upstream.nsi",
);

describe("W4-04 package NSIS hook placement", () => {
  it("includes package lifecycle hooks only after product defines and runtime vars", () => {
    const upstream = fs.readFileSync(upstreamPath, "utf8");
    const generated = buildFyluraNsisTemplate(upstream);

    const hook = '!include "{{installer_hooks}}"';
    const mainBinary = '!define MAINBINARYNAME "{{main_binary_name}}"';
    const passiveMode = "Var PassiveMode";
    const additionalPlugin = '!addplugindir "${ADDITIONALPLUGINSPATH}"';
    const welcomePage = "!insertmacro MUI_PAGE_WELCOME";

    const hookIndex = generated.indexOf(hook);
    expect(hookIndex).toBeGreaterThan(generated.indexOf(mainBinary));
    expect(hookIndex).toBeGreaterThan(generated.indexOf(passiveMode));
    expect(hookIndex).toBeGreaterThan(generated.indexOf(additionalPlugin));
    expect(hookIndex).toBeLessThan(generated.indexOf(welcomePage));
    expect(generated.indexOf(hook, hookIndex + hook.length)).toBe(-1);

    const upstreamHookIndex = upstream.indexOf(hook);
    expect(upstreamHookIndex).toBeLessThan(upstream.indexOf(mainBinary));
    expect(upstreamHookIndex).toBeLessThan(upstream.indexOf(passiveMode));
  });
});
