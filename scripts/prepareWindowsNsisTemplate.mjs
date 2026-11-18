import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
export const TAURI_NSIS_UPSTREAM_BLOB_SHA = "a48a46149f6d6bdc76a0bf13f53e4acdfedb310b";
export const upstreamTemplatePath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  "tauri-2.11.2-installer.upstream.nsi",
);
export const generatedTemplatePath = path.join(
  repositoryRoot,
  "src-tauri",
  "windows",
  ".generated",
  "fylura-installer.nsi",
);

const lines = (...values) => values.join("\n");

function canonicalizeUpstreamTemplate(source) {
  // Git stores this vendored template with LF. Windows may materialize the
  // same tracked blob with CRLF under core.autocrlf; normalize only line
  // endings so blob identity and exact anchors remain deterministic.
  return source.replace(/\r\n/g, "\n");
}

function gitBlobSha(content) {
  const bytes = Buffer.from(content, "utf8");
  return crypto
    .createHash("sha1")
    .update(`blob ${bytes.length}\0`)
    .update(bytes)
    .digest("hex");
}

function replaceExactly(source, before, after, label) {
  const first = source.indexOf(before);
  if (first < 0) {
    throw new Error(`Tauri NSIS template drift: missing ${label} anchor`);
  }
  if (source.indexOf(before, first + before.length) >= 0) {
    throw new Error(`Tauri NSIS template drift: duplicate ${label} anchor`);
  }
  return source.slice(0, first) + after + source.slice(first + before.length);
}

export function buildFyluraNsisTemplate(upstream) {
  const canonicalUpstream = canonicalizeUpstreamTemplate(upstream);
  const actualBlobSha = gitBlobSha(canonicalUpstream);
  if (actualBlobSha !== TAURI_NSIS_UPSTREAM_BLOB_SHA) {
    throw new Error(
      `Unexpected Tauri NSIS template blob ${actualBlobSha}; expected ${TAURI_NSIS_UPSTREAM_BLOB_SHA}`,
    );
  }

  let output = canonicalUpstream;

  output = replaceExactly(
    output,
    "!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.ConfirmShow",
    lines(
      "!define MUI_PAGE_CUSTOMFUNCTION_SHOW un.FyluraUserDataChoiceHandledBeforeUac",
      "Function un.FyluraUserDataChoiceHandledBeforeUac",
      "FunctionEnd",
    ),
    "route uninstall data choice through current user before UAC",
  );

  output = replaceExactly(
    output,
    lines(
      "Function un.ConfirmShow ; Add add a `Delete app data` check box",
      "  ; $1 inner dialog HWND",
      "  ; $2 window DPI",
      "  ; $3 style",
      "  ; $4 x",
      "  ; $5 y",
      "  ; $6 width",
      "  ; $7 height",
      '  FindWindow $1 "#32770" "" $HWNDPARENT ; Find inner dialog',
      '  System::Call "user32::GetDpiForWindow(p r1) i .r2"',
      '  ${If} $(^RTL) = 1',
      '    StrCpy $3 "${__NSD_CheckBox_EXSTYLE} | ${WS_EX_LAYOUTRTL}"',
      '    IntOp $4 50 * $2',
      '  ${Else}',
      '    StrCpy $3 "${__NSD_CheckBox_EXSTYLE}"',
      '    IntOp $4 0 * $2',
      '  ${EndIf}',
      '  IntOp $5 100 * $2',
      '  IntOp $6 400 * $2',
      '  IntOp $7 25 * $2',
      '  IntOp $4 $4 / 96',
      '  IntOp $5 $5 / 96',
      '  IntOp $6 $6 / 96',
      '  IntOp $7 $7 / 96',
      '  System::Call \'user32::CreateWindowEx(i r3, w "${__NSD_CheckBox_CLASS}", w "$(deleteAppData)", i ${__NSD_CheckBox_STYLE}, i r4, i r5, i r6, i r7, p r1, i0, i0, i0) i .s\'',
      '  Pop $DeleteAppDataCheckbox',
      '  SendMessage $HWNDPARENT ${WM_GETFONT} 0 0 $1',
      '  SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1',
      "FunctionEnd",
    ),
    lines("Function un.ConfirmShow", "FunctionEnd"),
    "remove elevated data checkbox implementation",
  );

  output = replaceExactly(
    output,
    lines("Var DeleteAppDataCheckbox", "Var DeleteAppDataCheckboxState"),
    "",
    "remove elevated data checkbox state",
  );

  output = replaceExactly(
    output,
    lines(
      "FunctionEnd",
      "!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave",
      "Function un.ConfirmLeave",
      "  SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState",
      "FunctionEnd",
    ),
    lines(
      "FunctionEnd",
      "!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave",
      "Function un.ConfirmLeave",
      "FunctionEnd",
    ),
    "remove elevated uninstall data choice",
  );

  output = replaceExactly(
    output,
    lines(
      '    SetShellVarContext current',
      '    RmDir /r "$APPDATA\\${BUNDLEID}"',
      '    RmDir /r "$LOCALAPPDATA\\${BUNDLEID}"',
    ),
    lines(
      '    SetShellVarContext current',
      '    ; The Fylura helper already verified app ownership and preserves Safe Trash.',
      '    ; Do not recursively remove either root here: it can contain real user files.',
    ),
    "prevent upstream recursive app-data deletion",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Delete app data if the checkbox is selected",
      "  ; and if not updating",
      "  ${If} $DeleteAppDataCheckboxState = 1",
      "  ${AndIf} $UpdateMode <> 1",
      "    ; Clear the install location $INSTDIR from registry",
      '    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"',
      '    DeleteRegKey /ifempty SHCTX "${MANUKEY}"',
      "",
      "    ; Clear the install language from registry",
      '    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"',
      '    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"',
      '    DeleteRegKey /ifempty HKCU "${MANUKEY}"',
      "",
      "    SetShellVarContext current",
      "    ; The Fylura helper already verified app ownership and preserves Safe Trash.",
      "    ; Do not recursively remove either root here: it can contain real user files.",
      "  ${EndIf}",
    ),
    "  ; Per-user cleanup runs in the initiating user's account before UAC.",
    "remove elevated per-user cleanup branch",
  );

  output = replaceExactly(
    output,
    lines(
      "!insertmacro MUI_RESERVEFILE_LANGDLL",
      "{{#each language_files}}",
      '  !include "{{this}}"',
      "{{/each}}",
    ),
    lines(
      "!insertmacro MUI_RESERVEFILE_LANGDLL",
      "{{#each language_files}}",
      '  !include "{{this}}"',
      "{{/each}}",
    ),
    "remove elevated uninstall data confirmation translations",
  );

  output = replaceExactly(
    output,
    lines(
      '  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode',
      "  ${IfNot} ${Errors}",
      "    StrCpy $UpdateMode 1",
      "  ${EndIf}",
      "FunctionEnd",
    ),
    lines(
      '  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode',
      "  ${IfNot} ${Errors}",
      "    StrCpy $UpdateMode 1",
      "  ${EndIf}",
      "FunctionEnd",
    ),
    "explicit silent uninstall data flags",
  );

  output = replaceExactly(
    output,
    lines(
      '      ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""',
      '      ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"',
      '      ${IfThen} $UpdateMode = 1 ${|} StrCpy $R1 "$R1 /UPDATE" ${|} ; append /UPDATE',
    ),
    lines(
      '      ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""',
      '      StrCpy $R1 "$\\\"$4\\uninstall.exe$\\\""',
      '      ${IfThen} $UpdateMode = 1 ${|} StrCpy $R1 "$R1 /UPDATE" ${|} ; append /UPDATE',
    ),
    "keep installer updates on the direct machine uninstaller",
  );

  output = replaceExactly(
    output,
    lines(
      '  ${If} $UpdateMode <> 1',
      '    DeleteRegValue HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "${PRODUCTNAME}"',
      '  ${EndIf}',
    ),
    "  ; Current-user autostart cleanup runs in the initiating user's account.",
    "move per-user autostart cleanup out of elevated NSIS",
  );

  output = replaceExactly(
    output,
    lines(
      "  !ifmacrodef NSIS_HOOK_PREINSTALL",
      "    !insertmacro NSIS_HOOK_PREINSTALL",
      "  !endif",
      "",
      '  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"',
    ),
    lines(
      "  ; Zen owns the reversible pre-file lifecycle. The service is stopped",
      "  ; through SCM before the name-only desktop process gate is evaluated.",
      "  Call ZCPrepareInstallLifecycle",
    ),
    "install pre-gate",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Copy main executable",
      '  File "${MAINBINARYSRCPATH}"',
    ),
    lines(
      "  ; Copy the main executable with deterministic non-interactive overwrite",
      "  ; semantics. The lifecycle enters the product-artifact-uncertain stage",
      "  ; before this destructive replacement attempt begins.",
      "  ClearErrors",
      "  SetOverwrite try",
      "  Call ZCMarkInstallIrreversible",
      '  File "${MAINBINARYSRCPATH}"',
      "  IfErrors zc_install_partial_failure",
      "  Call ZCMarkInstallGeneratedMutation",
    ),
    "install main file",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each resources_dirs}}",
      '    CreateDirectory "$INSTDIR\\\\{{this}}"',
      "  {{/each}}",
    ),
    lines(
      "  {{#each resources_dirs}}",
      "    ClearErrors",
      '    CreateDirectory "$INSTDIR\\\\{{this}}"',
      "    IfErrors zc_install_partial_failure",
      "  {{/each}}",
    ),
    "install resource directories",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each resources}}",
      '    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"',
      "  {{/each}}",
    ),
    lines(
      "  {{#each resources}}",
      '    !insertmacro ZC_INSTALL_RESOURCE "{{this.[1]}}" "{{no-escape @key}}"',
      "  {{/each}}",
    ),
    "install resources",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each binaries}}",
      '    File /a "/oname={{this}}" "{{no-escape @key}}"',
      "  {{/each}}",
    ),
    lines(
      "  {{#each binaries}}",
      "    ClearErrors",
      '    File /a "/oname={{this}}" "{{no-escape @key}}"',
      "    IfErrors zc_install_partial_failure",
      "  {{/each}}",
    ),
    "install external binaries",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Create uninstaller",
      '  WriteUninstaller "$INSTDIR\\uninstall.exe"',
      "",
      "  ; Save $INSTDIR in registry for future installations",
      '  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR',
    ),
    lines(
      "  ; Create uninstaller",
      "  ClearErrors",
      '  WriteUninstaller "$INSTDIR\\uninstall.exe"',
      "  IfErrors zc_install_partial_failure",
      "",
      "  ; Save $INSTDIR in registry for future installations",
      "  ClearErrors",
      '  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR',
      "  IfErrors zc_install_partial_failure",
    ),
    "install uninstaller and manufacturer metadata",
  );

  output = replaceExactly(
    output,
    lines(
      "  ${If} $OldMainBinaryName != \"\"",
      '  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"',
      '    Delete "$INSTDIR\\$OldMainBinaryName"',
      "  ${EndIf}",
    ),
    lines(
      "  ${If} $OldMainBinaryName != \"\"",
      '  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"',
      "    ClearErrors",
      '    Delete "$INSTDIR\\$OldMainBinaryName"',
      "    IfErrors zc_install_partial_failure",
      "  ${EndIf}",
    ),
    "install old-main cleanup",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Save current MAINBINARYNAME for future updates",
      '  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"',
      "",
      "  ; Registry information for add/remove programs",
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\""',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\\\"$INSTDIR$\\\""',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\\\"$INSTDIR\\uninstall.exe$\\\""',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"',
    ),
    lines(
      "  ; Save current MAINBINARYNAME and Add/Remove Programs metadata.",
      "  ClearErrors",
      '  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\""',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\\\"$INSTDIR$\\\""',
      '  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" --fylura-uninstall-launcher"',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"',
      "  IfErrors zc_install_partial_failure",
    ),
    "install ARP metadata",
  );

  output = replaceExactly(
    output,
    lines(
      "  !ifmacrodef NSIS_HOOK_POSTINSTALL",
      "    !insertmacro NSIS_HOOK_POSTINSTALL",
      "  !endif",
      "",
      "  ; Auto close this page for passive mode",
    ),
    lines(
      "  SetOverwrite on",
      "  Call ZCFinishInstallLifecycle",
      "  Goto zc_install_section_done",
      "",
      "zc_install_partial_failure:",
      "  SetOverwrite on",
      "  Call ZCFailInstallPartial",
      "  Abort",
      "",
      "zc_install_section_done:",
      "  ; Auto close this page for passive mode",
    ),
    "install synchronous failure labels",
  );

  output = replaceExactly(
    output,
    lines(
      "  !ifmacrodef NSIS_HOOK_PREUNINSTALL",
      "    !insertmacro NSIS_HOOK_PREUNINSTALL",
      "  !endif",
      "",
      '  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"',
    ),
    lines(
      "  ; Zen owns the reversible pre-delete lifecycle. The captured service is",
      "  ; stopped through SCM before the name-only desktop process gate.",
      "  Call un.ZCPrepareUninstallLifecycle",
    ),
    "uninstall pre-gate",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Delete the app directory and its content from disk",
      "  ; Copy main executable",
      '  Delete "$INSTDIR\\${MAINBINARYNAME}.exe"',
    ),
    lines(
      "  ; Delete the app directory and its content from disk. The main binary is",
      "  ; the irreversible boundary: before it is removed we can still restore the",
      "  ; exact captured Preview/service state.",
      "  ClearErrors",
      '  Delete "$INSTDIR\\${MAINBINARYNAME}.exe"',
      "  IfErrors zc_uninstall_reversible_failure",
      "  Call un.ZCMarkUninstallIrreversible",
      "  Call un.ZCMarkUninstallGeneratedMutation",
    ),
    "uninstall main delete",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each resources}}",
      '    Delete "$INSTDIR\\\\{{this.[1]}}"',
      "  {{/each}}",
    ),
    lines(
      "  {{#each resources}}",
      '    !insertmacro ZC_UNINSTALL_RESOURCE "{{this.[1]}}"',
      "  {{/each}}",
    ),
    "uninstall resources",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each binaries}}",
      '    Delete "$INSTDIR\\\\{{this}}"',
      "  {{/each}}",
    ),
    lines(
      "  {{#each binaries}}",
      "    ClearErrors",
      '    Delete "$INSTDIR\\\\{{this}}"',
      "    IfErrors zc_uninstall_partial_failure",
      "  {{/each}}",
    ),
    "uninstall binaries",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Delete uninstaller",
      '  Delete "$INSTDIR\\uninstall.exe"',
    ),
    lines(
      "  ; Delete uninstaller",
      "  ClearErrors",
      '  Delete "$INSTDIR\\uninstall.exe"',
      "  IfErrors zc_uninstall_partial_failure",
    ),
    "uninstall uninstaller",
  );

  output = replaceExactly(
    output,
    lines(
      "  !ifmacrodef NSIS_HOOK_POSTUNINSTALL",
      "    !insertmacro NSIS_HOOK_POSTUNINSTALL",
      "  !endif",
      "",
      "  ; Auto close if passive mode or updating",
    ),
    lines(
      "  Goto zc_uninstall_generated_success",
      "",
      "zc_uninstall_reversible_failure:",
      "  ${If} $ZC_LIFECYCLE_UNINSTALL_STAGE >= ${ZC_LIFECYCLE_STAGE_FILE_MUTATION}",
      "    Call un.ZCFailUninstallPartial",
      "  ${Else}",
      "    Call un.ZCFailUninstallReversible",
      "  ${EndIf}",
      "  Abort",
      "",
      "zc_uninstall_partial_failure:",
      "  Call un.ZCFailUninstallPartial",
      "  Abort",
      "",
      "zc_uninstall_generated_success:",
      "  !ifmacrodef NSIS_HOOK_POSTUNINSTALL",
      "    !insertmacro NSIS_HOOK_POSTUNINSTALL",
      "  !endif",
      "  Call un.ZCFinishUninstallLifecycle",
      "",
      "  ; Auto close if passive mode or updating",
    ),
    "uninstall synchronous failure labels",
  );

  for (const required of [
    "Call ZCPrepareInstallLifecycle",
    "SetOverwrite try",
    "IfErrors zc_install_partial_failure",
    "Call un.ZCPrepareUninstallLifecycle",
    "IfErrors zc_uninstall_reversible_failure",
    "--fylura-uninstall-launcher",
    "Per-user cleanup runs in the initiating user's account before UAC",
    "Current-user autostart cleanup runs in the initiating user's account",
    "IfErrors zc_uninstall_partial_failure",
  ]) {
    if (!output.includes(required)) {
      throw new Error(`Generated Zen NSIS template is missing ${required}`);
    }
  }

  return output;
}

export function prepareWindowsNsisTemplate() {
  const upstream = fs.readFileSync(upstreamTemplatePath, "utf8");
  const output = buildFyluraNsisTemplate(upstream);
  fs.mkdirSync(path.dirname(generatedTemplatePath), { recursive: true });
  fs.writeFileSync(generatedTemplatePath, output, "utf8");
  return generatedTemplatePath;
}

export function cleanWindowsNsisTemplate() {
  fs.rmSync(path.dirname(generatedTemplatePath), { recursive: true, force: true });
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  prepareWindowsNsisTemplate();
}
