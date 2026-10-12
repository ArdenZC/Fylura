Unicode true
!include "FileFunc.nsh"
!insertmacro GetSize
!addplugindir "/nsisplugins"
!ifndef MANUFACTURER
!define MANUFACTURER "startlan"
!endif
!ifndef PRODUCTNAME
!define PRODUCTNAME "Zen Canvas"
!endif
!ifndef VERSION
!define VERSION "0.1.40"
!endif
!ifndef HOMEPAGE
!define HOMEPAGE "https://example.invalid"
!endif
!ifndef MAINBINARYNAME
!define MAINBINARYNAME "zen-canvas.exe"
!endif
!ifndef UNINSTKEY
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Zen Canvas"
!endif
!ifndef MANUPRODUCTKEY
!define MANUPRODUCTKEY "Software\startlan\Zen Canvas"
!endif
!ifndef ESTIMATEDSIZE
!define ESTIMATEDSIZE "1"
!endif
!ifndef ATTEMPT_VALUE
!define ATTEMPT_VALUE "1"
!endif
!ifndef DESTINATION
!define DESTINATION "1"
!endif
!ifndef DONE_LABEL
!define DONE_LABEL "1"
!endif
!ifndef EXPECTED
!define EXPECTED "1"
!endif
!ifndef EXT
!define EXT "1"
!endif
!ifndef GetParent
!define GetParent "1"
!endif
!ifndef ID
!define ID "1"
!endif
!ifndef INDEX
!define INDEX "1"
!endif
!ifndef MAP_FUNCTION
!define MAP_FUNCTION "1"
!endif
!ifndef MODE
!define MODE "1"
!endif
!ifndef NAME
!define NAME "1"
!endif
!ifndef NOTIFY_FUNCTION
!define NOTIFY_FUNCTION "1"
!endif
!ifndef NSIS_MAX_STRLEN
!define NSIS_MAX_STRLEN "1"
!endif
!ifndef OLD_PRESENT
!define OLD_PRESENT "1"
!endif
!ifndef OLD_VALUE
!define OLD_VALUE "1"
!endif
!ifndef OWNER
!define OWNER "1"
!endif
!ifndef PATH
!define PATH "1"
!endif
!ifndef PREFIX
!define PREFIX "1"
!endif
!ifndef READ_FUNCTION
!define READ_FUNCTION "1"
!endif
!ifndef ROLLBACK_FUNCTION
!define ROLLBACK_FUNCTION "1"
!endif
!ifndef ROOT
!define ROOT "1"
!endif
!ifndef SERVICE_NAME
!define SERVICE_NAME "1"
!endif
!ifndef SOURCE
!define SOURCE "1"
!endif
!ifndef STALE_FUNCTION
!define STALE_FUNCTION "1"
!endif
!ifndef VALUE
!define VALUE "1"
!endif
Var PassiveMode
Var UpdateMode
Var WixMode
OutFile "/tmp/issue-380-u1r-gate0-e88afb1e5ba24428ba4541babe89d256/d1a-compile/d1a.exe"
!include "/tmp/issue-380-u1r-gate0-e88afb1e5ba24428ba4541babe89d256/d1a/src-tauri/windows/installer-lifecycle-wrapper.nsh"
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

Section "Issue 380 U1R offline macro expansion fixture" U1R_GATE0
  Call DetectZenCanvasPreexistingProduct
  Call ValidateZenCanvasPreexistingProduct
  Call ValidateZenCanvasPreviewCore
  Call ValidateZenCanvasIndexServiceOwnership
  WriteUninstaller "$TEMP\issue-380-u1r-gate0-uninstaller.exe"
SectionEnd
