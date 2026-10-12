import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  cleanWindowsNsisTemplate,
  generatedTemplatePath,
  prepareWindowsNsisTemplate,
} from "./prepareWindowsNsisLifecycleTemplate.mjs";
import { assertGeneratedPreviewResourcePathFile } from "./verifyWindowsNsisPreviewResource.mjs";

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
  "zen_canvas_windows_preview_handler.dll",
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
    if (process.env.ZC_U1R_NSIS_DIAGNOSTICS === "1") {
      let diagnostic = fs.readFileSync(generatedTemplatePath, "utf8");
      const inject = (before, after, label) => {
        if (diagnostic.indexOf(before) < 0 || diagnostic.indexOf(before, diagnostic.indexOf(before) + before.length) >= 0) {
          throw new Error(`U1R NSIS diagnostic anchor missing or duplicate: ${label}`);
        }
        diagnostic = diagnostic.replace(before, after);
      };
      inject("Function .onInit\n", "Function .onInit\n  DetailPrint \"[U1R] installer_on_init\"\n", "onInit logger");
      inject("Function PageReinstall\n", "Function PageReinstall\n  DetailPrint \"[U1R] PageReinstall passive=$PassiveMode update=$UpdateMode instdir=$INSTDIR\"\n", "maintenance page entry");
      inject("nsis_tauri_utils::SemverCompare \"${VERSION}\" $R0\n  Pop $R0\n", "nsis_tauri_utils::SemverCompare \"${VERSION}\" $R0\n  Pop $R0\n  DetailPrint \"[U1R] PageReinstall semver_compare=$R0\"\n", "maintenance page version result");
      inject("Function PageLeaveReinstall\n", "Function PageLeaveReinstall\n  DetailPrint \"[U1R] PageLeaveReinstall compare=$R0 update=$UpdateMode passive=$PassiveMode wix=$WixMode\"\n", "maintenance page leave");
      inject("  reinst_uninstall:\n    HideWindow\n", "  reinst_uninstall:\n    DetailPrint \"[U1R] maintenance_uninstall_begin compare=$R0 selection=$R1 update=$UpdateMode wix=$WixMode\"\n    HideWindow\n", "maintenance uninstall branch");
      inject("    BringToFront\n\n    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code\n", "    BringToFront\n    DetailPrint \"[U1R] maintenance_uninstaller_return exit=$0\"\n\n    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code\n", "maintenance uninstaller return");
      inject('      StrCpy $R1 "$R1 _?=$4" ; append uninstall directory\n', '      DetailPrint "[U1R] old_uninstaller_trace=$TEMP\\issue-380-u1r-old-uninstaller.log"\n      StrCpy $R1 "$R1 /LOG=$TEMP\\issue-380-u1r-old-uninstaller.log _?=$4" ; append uninstall directory\n', "old uninstaller diagnostic trace");
      inject("  Call ZCPrepareInstallLifecycleFinal", "  DetailPrint \"[U1R] lifecycle_prepare_final_begin\"\n  Call ZCPrepareInstallLifecycleFinal\n  DetailPrint \"[U1R] lifecycle_prepare_final_returned\"", "final lifecycle owner");
      if (!diagnostic.includes('DetailPrint "[U1R] installer_on_init"') ||
          !diagnostic.includes('DetailPrint "[U1R] lifecycle_prepare_final_begin"') ||
          /u1r_d1a_trace_\d+_|Call U1RWriteInstallerTrace/.test(diagnostic)) {
        throw new Error("U1R NSIS DetailPrint diagnostics are missing or contain unsafe generated trace labels/helper calls");
      }
      fs.writeFileSync(generatedTemplatePath, diagnostic, "utf8");
    }
    generatedNsisTemplate = true;
  }

  const tauriCommand = path.join(
    repositoryRoot,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "tauri.cmd" : "tauri",
  );
  const tauriArgs = ["build", "--features", "desktop-runtime"];
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
