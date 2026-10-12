!ifndef TRACE_MODE
  !define TRACE_MODE detail
!endif
!ifndef RUN_MODE
  !define RUN_MODE stage
!endif
!ifndef OUTPUT_NAME
  !define OUTPUT_NAME "issue380-s0-probe.exe"
!endif

Unicode True
Target x86-unicode
Name "Zen Canvas Issue 380 S0 isolated NSIS probe"
OutFile "${OUTPUT_NAME}"
RequestExecutionLevel user
InstallDir "$TEMP\issue380-s0-observability-probe"
SilentInstall silent
SilentUninstall silent

!define S0_REGISTRY_KEY "Software\ZenCanvasS0Harness"
!define S0_REGISTRY_VALUE "EstimatedSize"
!define S0_EXPECTED_DWORD 35150
!define S0_ACTUAL_DWORD 35149
!define S0_TRACE_FILENAME "direct-evidence.log"

!if "${TRACE_MODE}" == "file"
  !macro S0_DECLARE_TRACE_FUNCTION FUNCTION_NAME CONTEXT_NAME
    Function ${FUNCTION_NAME}
      ClearErrors
      FileOpen $R9 "$EXEDIR\${S0_TRACE_FILENAME}" a
      IfErrors trace_open_failed
      FileWrite $R9 "[S0] context=${CONTEXT_NAME} predicate=HKCU\${S0_REGISTRY_KEY}\${S0_REGISTRY_VALUE} type=REG_DWORD expected=${S0_EXPECTED_DWORD} actual=$R0 result=FAIL$\r$\n"
      FileClose $R9
      Goto trace_complete
    trace_open_failed:
      DetailPrint "[S0] trace_write_failed context=${CONTEXT_NAME}"
    trace_complete:
    FunctionEnd
  !macroend

  !insertmacro S0_DECLARE_TRACE_FUNCTION AppendInstallerTrace installer
  !insertmacro S0_DECLARE_TRACE_FUNCTION un.AppendUninstallerTrace uninstaller
!endif

Function .onInit
  SetRegView 64
  !if "${RUN_MODE}" == "stage"
    WriteRegDWORD HKCU "${S0_REGISTRY_KEY}" "${S0_REGISTRY_VALUE}" ${S0_ACTUAL_DWORD}
    IfErrors 0 +2
      SetErrorLevel 73
    Return
  !else
    DetailPrint "[S0] predicate=HKCU\${S0_REGISTRY_KEY}\${S0_REGISTRY_VALUE} type=REG_DWORD expected=${S0_EXPECTED_DWORD}"
    ClearErrors
    ReadRegDWORD $R0 HKCU "${S0_REGISTRY_KEY}" "${S0_REGISTRY_VALUE}"
    IfErrors registry_read_failed
    IntCmp $R0 ${S0_EXPECTED_DWORD} registry_match registry_mismatch registry_mismatch
    registry_read_failed:
      StrCpy $R0 "<unreadable>"
    registry_mismatch:
      DetailPrint "[S0] first_failure=EstimatedSize type=REG_DWORD expected=${S0_EXPECTED_DWORD} actual=$R0 result=FAIL"
      !if "${TRACE_MODE}" == "file"
        Call AppendInstallerTrace
      !endif
      SetErrorLevel 37
      Abort "S0 synthetic Registry DWORD mismatch"
    registry_match:
      SetErrorLevel 0
  !endif
FunctionEnd

Function un.onInit
  SetRegView 64
  DetailPrint "[S0] uninstaller predicate=HKCU\${S0_REGISTRY_KEY}\${S0_REGISTRY_VALUE} type=REG_DWORD expected=${S0_EXPECTED_DWORD}"
  ClearErrors
  ReadRegDWORD $R0 HKCU "${S0_REGISTRY_KEY}" "${S0_REGISTRY_VALUE}"
  IfErrors un_registry_read_failed
  IntCmp $R0 ${S0_EXPECTED_DWORD} un_registry_match un_registry_mismatch un_registry_mismatch
  un_registry_read_failed:
    StrCpy $R0 "<unreadable>"
  un_registry_mismatch:
    DetailPrint "[S0] uninstaller_first_failure=EstimatedSize type=REG_DWORD expected=${S0_EXPECTED_DWORD} actual=$R0 result=FAIL"
    !if "${TRACE_MODE}" == "file"
      Call un.AppendUninstallerTrace
    !endif
    SetErrorLevel 42
    Abort "S0 synthetic uninstaller Registry DWORD mismatch"
  un_registry_match:
    SetErrorLevel 0
FunctionEnd

Section "Stage isolated uninstaller"
  WriteUninstaller "$EXEDIR\generated-uninstaller.exe"
SectionEnd
