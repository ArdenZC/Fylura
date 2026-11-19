const lines = (...values) => values.join("\n");

function replaceExactly(source, before, after, label) {
  const first = source.indexOf(before);
  if (first < 0) {
    throw new Error(`Zen NSIS final orchestration drift: missing ${label} anchor`);
  }
  if (source.indexOf(before, first + before.length) >= 0) {
    throw new Error(`Zen NSIS final orchestration drift: duplicate ${label} anchor`);
  }
  return source.slice(0, first) + after + source.slice(first + before.length);
}

export function finalizeWindowsNsisLifecycleOrchestration(source) {
  let output = source;

  output = replaceExactly(
    output,
    "  Call ZCPrepareInstallLifecycle",
    "  Call ZCPrepareInstallLifecycleFinal",
    "final install prepare owner",
  );

  output = replaceExactly(
    output,
    "  Call un.ZCPrepareUninstallLifecycle",
    "  Call un.ZCPrepareUninstallLifecycleFinal",
    "final uninstall prepare owner",
  );

  output = replaceExactly(
    output,
    lines(
      "  SetOverwrite on",
      "  Call ZCFinishInstallLifecycle",
    ),
    lines(
      "  SetOverwrite on",
      "  Call ZCPostInstallLifecycleFinal",
      "  Call ZCFinishInstallLifecycle",
    ),
    "final install post owner",
  );

  output = replaceExactly(
    output,
    lines(
      "zc_uninstall_generated_success:",
      "  !ifmacrodef NSIS_HOOK_POSTUNINSTALL",
      "    !insertmacro NSIS_HOOK_POSTUNINSTALL",
      "  !endif",
      "  Call un.ZCFinishUninstallLifecycle",
    ),
    lines(
      "zc_uninstall_generated_success:",
      "  Call un.ZCPostUninstallLifecycleFinal",
      "  ${If} $ZC_UNINSTALL_FINAL_CLEAN != 1",
      "    SetErrorLevel 2",
      "    Quit",
      "  ${EndIf}",
      "  Call un.ZCFinishUninstallLifecycle",
    ),
    "final uninstall post owner",
  );

  for (const forbidden of [
    "!insertmacro NSIS_HOOK_PREINSTALL",
    "!insertmacro NSIS_HOOK_POSTINSTALL",
    "!insertmacro NSIS_HOOK_PREUNINSTALL",
    "!insertmacro NSIS_HOOK_POSTUNINSTALL",
  ]) {
    if (output.includes(forbidden)) {
      throw new Error(`Final Zen NSIS template still contains legacy execution owner ${forbidden}`);
    }
  }

  return output;
}
