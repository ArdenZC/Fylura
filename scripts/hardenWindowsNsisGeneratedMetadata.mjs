const lines = (...values) => values.join("\n");

function replaceExactly(source, before, after, label) {
  const first = source.indexOf(before);
  if (first < 0) {
    throw new Error(`Zen NSIS lifecycle template drift: missing ${label} anchor`);
  }
  if (source.indexOf(before, first + before.length) >= 0) {
    throw new Error(`Zen NSIS lifecycle template drift: duplicate ${label} anchor`);
  }
  return source.slice(0, first) + after + source.slice(first + before.length);
}

export function hardenWindowsNsisGeneratedMetadata(source) {
  let output = source;

  output = replaceExactly(
    output,
    lines(
      "  {{#each file_associations as |association| ~}}",
      "    {{#each association.ext as |ext| ~}}",
      '       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$INSTDIR\\${MAINBINARYNAME}.exe,0" "Open with ${PRODUCTNAME}" "$INSTDIR\\${MAINBINARYNAME}.exe $\\\"%1$\\\""',
      "    {{/each}}",
      "  {{/each}}",
    ),
    lines(
      "  {{#each file_associations as |association| ~}}",
      "    {{#each association.ext as |ext| ~}}",
      "       ClearErrors",
      '       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$INSTDIR\\${MAINBINARYNAME}.exe,0" "Open with ${PRODUCTNAME}" "$INSTDIR\\${MAINBINARYNAME}.exe $\\\"%1$\\\""',
      "       IfErrors zc_install_partial_failure",
      "    {{/each}}",
      "  {{/each}}",
    ),
    "install file associations",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each deep_link_protocols as |protocol| ~}}",
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}" "URL Protocol" ""',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}" "" "URL:${BUNDLEID} protocol"',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}\\DefaultIcon" "" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\",0"',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}\\shell\\open\\command" "" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" $\\\"%1$\\\""',
      "  {{/each}}",
    ),
    lines(
      "  {{#each deep_link_protocols as |protocol| ~}}",
      "    ClearErrors",
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}" "URL Protocol" ""',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}" "" "URL:${BUNDLEID} protocol"',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}\\DefaultIcon" "" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\",0"',
      '    WriteRegStr SHCTX "Software\\Classes\\\\{{protocol}}\\shell\\open\\command" "" "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" $\\\"%1$\\\""',
      "    IfErrors zc_install_partial_failure",
      "  {{/each}}",
    ),
    "install deep links",
  );

  output = replaceExactly(
    output,
    lines(
      '  !if "${INSTALLMODE}" == "both"',
      "    ; Save install mode to be selected by default for the next installation such as updating",
      "    ; or when uninstalling",
      '    WriteRegStr SHCTX "${UNINSTKEY}" $MultiUser.InstallMode 1',
      "  !endif",
    ),
    lines(
      '  !if "${INSTALLMODE}" == "both"',
      "    ; Save install mode to be selected by default for the next installation such as updating",
      "    ; or when uninstalling",
      "    ClearErrors",
      '    WriteRegStr SHCTX "${UNINSTKEY}" $MultiUser.InstallMode 1',
      "    IfErrors zc_install_partial_failure",
      "  !endif",
    ),
    "install multi-user metadata",
  );

  output = replaceExactly(
    output,
    lines(
      '  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2',
      "  IntOp $0 $0 + ${ESTIMATEDSIZE}",
      '  IntFmt $0 "0x%08X" $0',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"',
    ),
    lines(
      "  ClearErrors",
      '  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2',
      "  IntOp $0 $0 + ${ESTIMATEDSIZE}",
      '  IntFmt $0 "0x%08X" $0',
      '  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"',
      "  IfErrors zc_install_partial_failure",
    ),
    "install estimated size metadata",
  );

  output = replaceExactly(
    output,
    lines(
      '  !if "${HOMEPAGE}" != ""',
      '    WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"',
      '    WriteRegStr SHCTX "${UNINSTKEY}" "URLUpdateInfo" "${HOMEPAGE}"',
      '    WriteRegStr SHCTX "${UNINSTKEY}" "HelpLink" "${HOMEPAGE}"',
      "  !endif",
    ),
    lines(
      '  !if "${HOMEPAGE}" != ""',
      "    ClearErrors",
      '    WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"',
      '    WriteRegStr SHCTX "${UNINSTKEY}" "URLUpdateInfo" "${HOMEPAGE}"',
      '    WriteRegStr SHCTX "${UNINSTKEY}" "HelpLink" "${HOMEPAGE}"',
      "    IfErrors zc_install_partial_failure",
      "  !endif",
    ),
    "install homepage metadata",
  );

  output = replaceExactly(
    output,
    lines(
      "  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application",
      "    Call CreateOrUpdateStartMenuShortcut",
      "  !insertmacro MUI_STARTMENU_WRITE_END",
    ),
    lines(
      "  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application",
      "    ClearErrors",
      "    Call CreateOrUpdateStartMenuShortcut",
      "    IfErrors zc_install_partial_failure",
      "  !insertmacro MUI_STARTMENU_WRITE_END",
    ),
    "install start-menu shortcut",
  );

  output = replaceExactly(
    output,
    lines(
      "  ${If} $PassiveMode = 1",
      "  ${OrIf} ${Silent}",
      "    Call CreateOrUpdateDesktopShortcut",
      "  ${EndIf}",
    ),
    lines(
      "  ${If} $PassiveMode = 1",
      "  ${OrIf} ${Silent}",
      "    ClearErrors",
      "    Call CreateOrUpdateDesktopShortcut",
      "    IfErrors zc_install_partial_failure",
      "  ${EndIf}",
    ),
    "install desktop shortcut",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each file_associations as |association| ~}}",
      "    {{#each association.ext as |ext| ~}}",
      '      !insertmacro APP_UNASSOCIATE "{{ext}}" "{{or association.name ext}}"',
      "    {{/each}}",
      "  {{/each}}",
    ),
    lines(
      "  {{#each file_associations as |association| ~}}",
      "    {{#each association.ext as |ext| ~}}",
      "      ClearErrors",
      '      !insertmacro APP_UNASSOCIATE "{{ext}}" "{{or association.name ext}}"',
      "      IfErrors zc_uninstall_partial_failure",
      "    {{/each}}",
      "  {{/each}}",
    ),
    "uninstall file associations",
  );

  output = replaceExactly(
    output,
    lines(
      '    ${If} $R7 == "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" $\\\"%1$\\\""',
      '      DeleteRegKey SHCTX "Software\\Classes\\\\{{protocol}}"',
      "    ${EndIf}",
    ),
    lines(
      '    ${If} $R7 == "$\\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\\" $\\\"%1$\\\""',
      "      ClearErrors",
      '      DeleteRegKey SHCTX "Software\\Classes\\\\{{protocol}}"',
      "      IfErrors zc_uninstall_partial_failure",
      "    ${EndIf}",
    ),
    "uninstall deep links",
  );

  output = replaceExactly(
    output,
    lines(
      "  {{#each resources_ancestors}}",
      '  RMDir /REBOOTOK "$INSTDIR\\\\{{this}}"',
      "  {{/each}}",
      '  RMDir "$INSTDIR"',
    ),
    lines(
      "  {{#each resources_ancestors}}",
      "  ClearErrors",
      '  RMDir "$INSTDIR\\\\{{this}}"',
      "  IfErrors zc_uninstall_partial_failure",
      "  {{/each}}",
      "  ClearErrors",
      '  RMDir "$INSTDIR"',
      "  IfErrors zc_uninstall_partial_failure",
    ),
    "uninstall install-directory cleanup",
  );

  output = replaceExactly(
    output,
    lines(
      "  ; Remove registry information for add/remove programs",
      '  !if "${INSTALLMODE}" == "both"',
      '    DeleteRegKey SHCTX "${UNINSTKEY}"',
      '  !else if "${INSTALLMODE}" == "perMachine"',
      '    DeleteRegKey HKLM "${UNINSTKEY}"',
      "  !else",
      '    DeleteRegKey HKCU "${UNINSTKEY}"',
      "  !endif",
    ),
    lines(
      "  ; Remove registry information for add/remove programs",
      "  ; Use the exact-owned 64-bit helper for this per-machine package.",
      "  Call un.RemoveFyluraUninstallRegistration",
      "  ${If} $ZC_UNINSTALL_REGISTRATION_CLEAN != 1",
      "    Call un.ZCFailUninstallPartial",
      "    Abort",
      "  ${EndIf}",
    ),
    "uninstall ARP metadata",
  );

  for (const required of [
    "APP_ASSOCIATE",
    "IfErrors zc_install_partial_failure",
    "APP_UNASSOCIATE",
    "IfErrors zc_uninstall_partial_failure",
    'RMDir "$INSTDIR"',
  ]) {
    if (!output.includes(required)) {
      throw new Error(`Generated Zen NSIS metadata template is missing ${required}`);
    }
  }

  return output;
}
