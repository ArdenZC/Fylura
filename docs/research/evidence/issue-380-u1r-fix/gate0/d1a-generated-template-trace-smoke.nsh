Function .onInit
  DetailPrint "[U1R] installer_on_init"
FunctionEnd
Function PageReinstall
  DetailPrint "[U1R] PageReinstall passive=$PassiveMode update=$UpdateMode instdir=$INSTDIR"
  nsis_tauri_utils::SemverCompare "${VERSION}" $R0
  Pop $R0
  DetailPrint "[U1R] PageReinstall semver_compare=$R0"
  reinst_uninstall:
    DetailPrint "[U1R] maintenance_uninstall_begin compare=$R0 selection=$R1 update=$UpdateMode wix=$WixMode"
    HideWindow
    BringToFront
    DetailPrint "[U1R] maintenance_uninstaller_return exit=$0"

    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code
      DetailPrint "[U1R] old_uninstaller_trace=$TEMP\issue-380-u1r-old-uninstaller.log"
      StrCpy $R1 "$R1 /LOG=$TEMP\issue-380-u1r-old-uninstaller.log _?=$4" ; append uninstall directory
FunctionEnd
Function PageLeaveReinstall
  DetailPrint "[U1R] PageLeaveReinstall compare=$R0 update=$UpdateMode passive=$PassiveMode wix=$WixMode"
  DetailPrint "[U1R] lifecycle_prepare_final_begin"
  Call ZCPrepareInstallLifecycleFinal
  DetailPrint "[U1R] lifecycle_prepare_final_returned"
FunctionEnd
