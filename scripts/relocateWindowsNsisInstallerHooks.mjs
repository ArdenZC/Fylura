const HOOK_BLOCK = [
  "{{#if installer_hooks}}",
  '!include "{{installer_hooks}}"',
  "{{/if}}",
].join("\n");

function replaceExactly(source, before, after, label) {
  const first = source.indexOf(before);
  if (first < 0) {
    throw new Error(`Zen NSIS hook relocation drift: missing ${label} anchor`);
  }
  if (source.indexOf(before, first + before.length) >= 0) {
    throw new Error(`Zen NSIS hook relocation drift: duplicate ${label} anchor`);
  }
  return source.slice(0, first) + after + source.slice(first + before.length);
}

export function relocateWindowsNsisInstallerHooks(source) {
  let output = replaceExactly(
    source,
    HOOK_BLOCK,
    "; Fylura relocates installer hooks after product defines and runtime Vars.",
    "upstream installer hook block",
  );

  output = replaceExactly(
    output,
    [
      "Var PassiveMode",
      "Var UpdateMode",
      "Var NoShortcutMode",
      "Var WixMode",
      "Var OldMainBinaryName",
    ].join("\n"),
    [
      "Var PassiveMode",
      "Var UpdateMode",
      "Var NoShortcutMode",
      "Var WixMode",
      "Var OldMainBinaryName",
      "",
      "; W4-04 package lifecycle helpers intentionally compile only after",
      "; MAINBINARYNAME/PRODUCTNAME/VERSION and PassiveMode are declared.",
      "; The hook include is inserted after NSIS additional plugins below.",
    ].join("\n"),
    "runtime variable declaration block",
  );

  output = replaceExactly(
    output,
    ["# additional plugins", '!addplugindir "${ADDITIONALPLUGINSPATH}"'].join("\n"),
    [
      "# additional plugins",
      '!addplugindir "${ADDITIONALPLUGINSPATH}"',
      "",
      "; W4-04 package lifecycle helpers compile after the Tauri plugin path",
      "; is registered so nsis_tauri_utils calls resolve during NSIS compilation.",
      HOOK_BLOCK,
    ].join("\n"),
    "additional plugin path block",
  );

  const includeIndex = output.indexOf('!include "{{installer_hooks}}"');
  const mainBinaryIndex = output.indexOf('!define MAINBINARYNAME "{{main_binary_name}}"');
  const passiveVarIndex = output.indexOf("Var PassiveMode");
  const additionalPluginIndex = output.indexOf('!addplugindir "${ADDITIONALPLUGINSPATH}"');
  const welcomePageIndex = output.indexOf("!insertmacro MUI_PAGE_WELCOME");
  if (
    includeIndex <= mainBinaryIndex ||
    includeIndex <= passiveVarIndex ||
    includeIndex <= additionalPluginIndex ||
    welcomePageIndex < 0 ||
    includeIndex >= welcomePageIndex
  ) {
    throw new Error(
      "Relocated Zen NSIS installer hooks are outside the required define/Var/plugin/page boundary",
    );
  }

  return output;
}
