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
    lines(
      "Var DeleteAppDataCheckbox",
      "Var DeleteAppDataCheckboxState",
    ),
    lines(
      "Var DeleteAppDataCheckbox",
      "Var DeleteAppDataCheckboxState",
      "Var DeleteAppDataConfirmationState",
    ),
    "uninstall data choice state",
  );

  output = replaceExactly(
    output,
    lines(
      '  SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1',
      "FunctionEnd",
      "!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave",
      "Function un.ConfirmLeave",
      "  SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState",
      "FunctionEnd",
    ),
    lines(
      '  SendMessage $DeleteAppDataCheckbox ${WM_SETFONT} $1 1',
      "  ${If} $DeleteAppDataCheckboxState = 1",
      "    SendMessage $DeleteAppDataCheckbox ${BM_SETCHECK} 1 0",
      "  ${EndIf}",
      "FunctionEnd",
      "!define MUI_PAGE_CUSTOMFUNCTION_LEAVE un.ConfirmLeave",
      "Function un.ConfirmLeave",
      "  SendMessage $DeleteAppDataCheckbox ${BM_GETCHECK} 0 0 $DeleteAppDataCheckboxState",
      "  ${If} $DeleteAppDataCheckboxState = 1",
      "  ${AndIf} $UpdateMode <> 1",
      "  ${AndIf} $DeleteAppDataConfirmationState <> 1",
      "    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 \"$(confirmFyluraDataDeletion)\" IDYES +2",
      "    Abort",
      "    StrCpy $DeleteAppDataConfirmationState 1",
      "  ${EndIf}",
      "FunctionEnd",
    ),
    "explicit uninstall data confirmation",
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
      'LangString confirmFyluraDataDeletion ${LANG_ENGLISH} "This removes this Windows user\'s Fylura settings, database, index, cache, logs, and API credentials. History/Restore records will be removed. Safe Trash and organized files are kept. Continue?"',
      'LangString confirmFyluraDataDeletion ${LANG_SIMPCHINESE} "这会删除此 Windows 用户的 Fylura 设置、数据库、索引、缓存、日志和 API 凭据。History/Restore 恢复记录会一并删除。Safe Trash 和已整理的文件会保留。是否继续？"',
    ),
    "uninstall data confirmation translations",
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
      '  ${GetOptions} $CMDLINE "/DELETE_APP_DATA" $R0',
      "  ${IfNot} ${Errors}",
      "    StrCpy $DeleteAppDataCheckboxState 1",
      '    ${GetOptions} $CMDLINE "/CONFIRM_DELETE_APP_DATA" $R1',
      "    ${IfNot} ${Errors}",
      "      StrCpy $DeleteAppDataConfirmationState 1",
      "    ${ElseIf} ${Silent}",
      "      SetErrorLevel 2",
      "      Abort",
      "    ${EndIf}",
      "  ${EndIf}",
      "FunctionEnd",
    ),
    "explicit silent uninstall data flags",
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
      '  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\\\"$INSTDIR\\uninstall.exe$\\\""',
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
      "  ${If} $DeleteAppDataCheckboxState = 1",
      "  ${AndIf} $UpdateMode <> 1",
      "    ; The user confirmed full cleanup. Mark this as irreversible before",
      "    ; credentials or the app database are removed.",
      "    Call un.ZCMarkUninstallIrreversible",
      '    nsExec::ExecToStack \'"$INSTDIR\\${MAINBINARYNAME}.exe" --fylura-uninstall-clean-user-data\'',
      "    Pop $0",
      "    Pop $1",
      "    ${If} $0 != 0",
      "      DetailPrint $1",
      '      MessageBox MB_ICONSTOP|MB_OK "Fylura app data cleanup could not be verified. $1" /SD IDOK',
      "      Call un.ZCFailUninstallPartial",
      "      Abort",
      "    ${EndIf}",
      "  ${EndIf}",
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
    "--fylura-uninstall-clean-user-data",
    '"/DELETE_APP_DATA"',
    '"/CONFIRM_DELETE_APP_DATA"',
    "Do not recursively remove either root",
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
